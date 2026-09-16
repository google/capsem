use super::*;
use capsem_api::stream::{decode_client_frame, encode_data, encode_status, ClientFrame, StreamKind};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

/// A fake service socket that checks the upgrade, then runs `script`.
fn fake_service<F, Fut>(dir: &std::path::Path, script: F) -> (PathBuf, tokio::task::JoinHandle<()>)
where
    F: FnOnce(tokio_tungstenite::WebSocketStream<UnixStream>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let path = dir.join("service.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let handle = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        #[allow(clippy::result_large_err)] // tungstenite's handshake callback signature
        let callback = |request: &Request, mut response: Response| {
            assert_eq!(request.uri().path(), "/vms/vm-1/stream");
            assert_eq!(request.headers()["sec-websocket-protocol"], "capsem.stream.v1");
            response
                .headers_mut()
                .insert("sec-websocket-protocol", HeaderValue::from_static("capsem.stream.v1"));
            Ok(response)
        };
        let socket = tokio_tungstenite::accept_hdr_async(socket, callback).await.unwrap();
        script(socket).await;
    });
    (path, handle)
}

async fn expect_start(socket: &mut tokio_tungstenite::WebSocketStream<UnixStream>) -> StreamControl {
    let Some(Ok(Message::Binary(bytes))) = socket.next().await else {
        panic!("expected start")
    };
    match decode_client_frame(&bytes).unwrap() {
        ClientFrame::Control(control) => control,
        other => panic!("expected control, got {other:?}"),
    }
}

#[tokio::test]
async fn a_stream_starts_then_delivers_output_and_exit() {
    let dir = tempfile::tempdir().unwrap();
    let (path, service) = fake_service(dir.path(), |mut socket| async move {
        assert_eq!(
            expect_start(&mut socket).await,
            StreamControl::Start {
                kind: StreamKind::Terminal,
                command: None
            }
        );
        socket
            .send(Message::Binary(encode_status(&StreamStatus::Started).into()))
            .await
            .unwrap();
        let Some(Ok(Message::Binary(bytes))) = socket.next().await else {
            panic!("expected stdin")
        };
        assert_eq!(
            decode_client_frame(&bytes).unwrap(),
            ClientFrame::Stdin(b"capsem-doctor\n")
        );
        let Some(Ok(Message::Binary(bytes))) = socket.next().await else {
            panic!("expected stdin EOF")
        };
        assert_eq!(
            decode_client_frame(&bytes).unwrap(),
            ClientFrame::Control(StreamControl::CloseStdin)
        );
        socket
            .send(Message::Binary(
                encode_data(StreamChannel::Stderr, b"diagnostic").into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Binary(
                encode_data(StreamChannel::Stdout, b"RESULT: PASS\xff").into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Binary(
                encode_status(&StreamStatus::Exit {
                    code: 0,
                    truncated: false,
                })
                .into(),
            ))
            .await
            .unwrap();
    });
    let client = UdsClient::new(path, false);
    let mut attached = client
        .open_stream(
            "vm-1",
            StreamControl::Start {
                kind: StreamKind::Terminal,
                command: None,
            },
        )
        .await
        .unwrap();
    attached.send_stdin(b"capsem-doctor\n").await.unwrap();
    attached.close_stdin().await.unwrap();
    assert_eq!(
        attached.next().await.unwrap(),
        StreamEvent::ErrorOutput(b"diagnostic".to_vec())
    );
    assert_eq!(
        attached.next().await.unwrap(),
        StreamEvent::Output(b"RESULT: PASS\xff".to_vec())
    );
    assert_eq!(
        attached.next().await.unwrap(),
        StreamEvent::Exit {
            code: 0,
            truncated: false
        }
    );
    service.await.unwrap();
}

#[tokio::test]
async fn a_refused_or_ended_stream_is_an_error_with_the_services_reason() {
    let dir = tempfile::tempdir().unwrap();
    let (path, service) = fake_service(dir.path(), |mut socket| async move {
        expect_start(&mut socket).await;
        socket
            .send(Message::Binary(
                encode_status(&StreamStatus::Error {
                    message: "not staged for attach".into(),
                })
                .into(),
            ))
            .await
            .unwrap();
    });
    let client = UdsClient::new(path, false);
    let error = client
        .open_stream(
            "vm-1",
            StreamControl::Start {
                kind: StreamKind::Container,
                command: None,
            },
        )
        .await
        .err()
        .expect("a refused stream is an error");
    assert!(error.to_string().contains("not staged for attach"), "{error:#}");
    service.await.unwrap();
}
