use super::*;
use capsem_api::stream::{decode_client_frame, encode_data, encode_status, ClientFrame, StreamKind};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{Callback, ErrorResponse, Request, Response};

// The upstream trait owns the handshake error type. Implementing its contract
// avoids introducing a closure with a suppressible large-error result.
struct AuthenticatedHandshake;

impl Callback for AuthenticatedHandshake {
    fn on_request(self, request: &Request, mut response: Response) -> std::result::Result<Response, ErrorResponse> {
        assert_eq!(request.uri().path(), "/prefix/vms/vm%20%C3%A9/stream");
        assert_eq!(request.headers()["authorization"], "Bearer private-token");
        assert_eq!(request.headers()["sec-websocket-protocol"], stream::STREAM_SUBPROTOCOL);
        response.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_static(stream::STREAM_SUBPROTOCOL),
        );
        Ok(response)
    }
}

async fn gateway<F, Fut>(script: F) -> (Transport, tokio::task::JoinHandle<()>)
where
    F: FnOnce(WebSocketStream<TcpStream>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/prefix", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        script(
            tokio_tungstenite::accept_hdr_async(socket, AuthenticatedHandshake)
                .await
                .unwrap(),
        )
        .await;
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
            .send(Message::Binary(encode_status(&StreamStatus::Started).unwrap().into()))
            .await
            .unwrap();
        let Some(Ok(Message::Binary(focus))) = socket.next().await else {
            panic!("focus")
        };
        assert_eq!(
            decode_client_frame(&focus).unwrap(),
            ClientFrame::Control(StreamControl::Focus { cols: 80, rows: 24 })
        );
        socket
            .send(Message::Binary(
                encode_status(&StreamStatus::Terminal {
                    state: Box::new(terminal_state()),
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let Some(Ok(Message::Binary(resize))) = socket.next().await else {
            panic!("resize")
        };
        assert_eq!(
            decode_client_frame(&resize).unwrap(),
            ClientFrame::Control(StreamControl::Resize { cols: 101, rows: 33 })
        );
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
                encode_status(&StreamStatus::Terminal {
                    state: Box::new(terminal_state()),
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Binary(
                encode_data(StreamChannel::Stderr, b"reason\xff").unwrap().into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Binary(
                encode_data(StreamChannel::Stdout, b"\0\xfe").unwrap().into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Binary(
                encode_status(&StreamStatus::Exit {
                    code: 7,
                    truncated: true,
                })
                .unwrap()
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
    attached
        .control(&StreamControl::Resize { cols: 101, rows: 33 })
        .await
        .unwrap();
    let StreamEvent::Terminal { state } = attached.next().await.unwrap() else {
        panic!("expected terminal metadata");
    };
    assert_eq!(state.title, "Agent");
    attached.send_stdin(b"\0\xff\n").await.unwrap();
    attached.close_stdin().await.unwrap();
    assert_eq!(
        attached.next_output().await.unwrap(),
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
                .unwrap()
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
            .send(Message::Binary(encode_status(&StreamStatus::Started).unwrap().into()))
            .await
            .unwrap();
        socket
            .send(Message::Binary(vec![0; stream::MAX_STREAM_FRAME_BYTES + 1].into()))
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

fn terminal_state() -> capsem_api::stream::TerminalState {
    serde_json::from_value(serde_json::json!({
        "revision": 1, "reset": true, "cols": 80, "rows": 24,
        "cursor": {"row": 0, "col": 0, "hidden": false},
        "modes": {"alternate_screen": false, "application_cursor": false, "application_keypad": false, "bracketed_paste": false, "mouse_protocol": "None", "mouse_encoding": "Default"},
        "title": "Agent", "icon_name": "", "bell_count": 0, "last_notification": null, "events": []
    })).unwrap()
}
