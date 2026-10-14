use super::*;
use capsem_api::stream::{decode_client_frame, encode_data, encode_status, ClientFrame, StreamKind};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

async fn gateway<F, Fut>(script: F) -> (Transport, tokio::task::JoinHandle<()>)
where
    F: FnOnce(WebSocketStream<TcpStream>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/prefix", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        #[allow(clippy::result_large_err)] // tungstenite owns this callback signature
        let check = |request: &Request, mut response: Response| {
            assert_eq!(request.uri().path(), "/prefix/vms/vm%20%C3%A9/stream");
            assert_eq!(request.headers()["authorization"], "Bearer private-token");
            assert_eq!(request.headers()["sec-websocket-protocol"], stream::STREAM_SUBPROTOCOL);
            response.headers_mut().insert(
                "sec-websocket-protocol",
                HeaderValue::from_static(stream::STREAM_SUBPROTOCOL),
            );
            Ok(response)
        };
        script(tokio_tungstenite::accept_hdr_async(socket, check).await.unwrap()).await;
    });
    (
        Transport::new(&url, "private-token", Duration::from_secs(2)).unwrap(),
        task,
    )
}

fn start() -> StreamControl {
    StreamControl::Start {
        kind: StreamKind::Terminal,
        command: None,
    }
}

#[tokio::test]
async fn authenticated_stream_preserves_binary_input_output_and_exit() {
    let (transport, task) = gateway(|mut socket| async move {
        let Some(Ok(Message::Binary(bytes))) = socket.next().await else {
            panic!("start")
        };
        assert_eq!(decode_client_frame(&bytes).unwrap(), ClientFrame::Control(start()));
        socket
            .send(Message::Binary(encode_status(&StreamStatus::Started).into()))
            .await
            .unwrap();
        let Some(Ok(Message::Binary(bytes))) = socket.next().await else {
            panic!("stdin")
        };
        assert_eq!(decode_client_frame(&bytes).unwrap(), ClientFrame::Stdin(b"\0\xff\n"));
        let Some(Ok(Message::Binary(bytes))) = socket.next().await else {
            panic!("close stdin")
        };
        assert_eq!(
            decode_client_frame(&bytes).unwrap(),
            ClientFrame::Control(StreamControl::CloseStdin)
        );
        socket
            .send(Message::Binary(
                encode_data(StreamChannel::Stderr, b"reason\xff").into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Binary(encode_data(StreamChannel::Stdout, b"\0\xfe").into()))
            .await
            .unwrap();
        socket
            .send(Message::Binary(
                encode_status(&StreamStatus::Exit {
                    code: 7,
                    truncated: true,
                })
                .into(),
            ))
            .await
            .unwrap();
    })
    .await;
    let mut attached = transport
        .open_stream("vm é", start(), CallOptions::default())
        .await
        .unwrap();
    attached.send_stdin(b"\0\xff\n").await.unwrap();
    attached.close_stdin().await.unwrap();
    assert_eq!(
        attached.next().await.unwrap(),
        StreamEvent::ErrorOutput(b"reason\xff".to_vec())
    );
    assert_eq!(attached.next().await.unwrap(), StreamEvent::Output(b"\0\xfe".to_vec()));
    assert_eq!(
        attached.next().await.unwrap(),
        StreamEvent::Exit {
            code: 7,
            truncated: true
        }
    );
    task.await.unwrap();
}

#[tokio::test]
async fn startup_denial_preserves_the_gateways_reason() {
    let (transport, task) = gateway(|mut socket| async move {
        socket.next().await.unwrap().unwrap();
        socket
            .send(Message::Binary(
                encode_status(&StreamStatus::Error {
                    message: "session retired".into(),
                })
                .into(),
            ))
            .await
            .unwrap();
    })
    .await;
    let error = transport
        .open_stream("vm é", start(), CallOptions::default())
        .await
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "gateway stream failed: session retired");
    task.await.unwrap();
}

#[tokio::test]
async fn missing_start_receipt_times_out_and_closes_the_socket() {
    let (transport, task) = gateway(|mut socket| async move {
        socket.next().await.unwrap().unwrap();
        assert!(matches!(socket.next().await, None | Some(Err(_))));
    })
    .await;
    let error = transport
        .open_stream(
            "vm é",
            start(),
            CallOptions {
                timeout: Some(Duration::from_millis(100)),
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "gateway stream failed: gateway stream startup timed out"
    );
    task.await.unwrap();
}

#[tokio::test]
async fn rejected_upgrade_is_not_retried_and_retains_http_status() {
    let mut gateway = crate::test_gateway::Server::reply(401, br#"{"error":"bad token"}"#, None).await;
    let transport = gateway.client();
    let error = transport
        .open_stream("vm-1", start(), CallOptions::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(error, Error::Http { status: 401, .. }));
    let (request, _) = gateway.received.recv().await.unwrap();
    assert_eq!(request.headers["authorization"], "Bearer private-token");
    assert!(gateway.received.try_recv().is_err());
}

#[tokio::test]
async fn frames_over_the_public_limit_are_refused() {
    let (transport, task) = gateway(|mut socket| async move {
        socket.next().await.unwrap().unwrap();
        socket
            .send(Message::Binary(encode_status(&StreamStatus::Started).into()))
            .await
            .unwrap();
        socket
            .send(Message::Binary(
                encode_data(StreamChannel::Stdout, &vec![0; stream::MAX_STREAM_FRAME_BYTES]).into(),
            ))
            .await
            .unwrap();
    })
    .await;
    let mut attached = transport
        .open_stream("vm é", start(), CallOptions::default())
        .await
        .unwrap();
    assert!(matches!(attached.next().await, Err(Error::Stream(_))));
    task.await.unwrap();
}
