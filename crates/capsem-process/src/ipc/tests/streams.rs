use super::*;

async fn connect_as(
    dispatcher: &Dispatcher,
    peer: &'static str,
) -> (Sender<ServiceToProcess>, Receiver<ProcessToService>) {
    let (process_stream, service_stream) = tokio::net::UnixStream::pair().unwrap();
    let _handler = dispatcher.connection(process_stream);
    let mut service_stream = service_stream.into_std().unwrap();
    tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_initiator(&mut service_stream, peer, "").unwrap();
        channel_from_std::<ServiceToProcess, ProcessToService>(service_stream).unwrap()
    })
    .await
    .unwrap()
}

/// A stream connection carries one stream's bytes. Lifecycle broadcasts meant
/// for command connections used to interleave with them.
#[tokio::test]
async fn stream_role_connections_receive_no_lifecycle_broadcasts() {
    let temp = tempfile::tempdir().unwrap();
    let (dispatcher, _ctrl_rx) = Dispatcher::new(temp.path());
    let _other_subscriber = dispatcher.events_tx.subscribe();
    let (tx, rx) = connect_as(&dispatcher, capsem_proto::handshake::STREAM_PEER_ID).await;
    // Give the connection time to subscribe if it (wrongly) does.
    tokio::time::sleep(Duration::from_millis(50)).await;
    dispatcher
        .events_tx
        .send(ProcessToService::StateChanged {
            id: "vm".into(),
            state: "Suspended".into(),
            trigger: "t".into(),
        })
        .unwrap();
    tx.send(ServiceToProcess::Ping).await.unwrap();
    assert!(
        matches!(rx.recv().await.unwrap(), ProcessToService::Pong),
        "a broadcast reached a stream connection"
    );
}

#[tokio::test]
async fn command_connections_still_receive_lifecycle_broadcasts() {
    let temp = tempfile::tempdir().unwrap();
    let (dispatcher, _ctrl_rx) = Dispatcher::new(temp.path());
    let (_tx, rx) = connect_as(&dispatcher, "capsem-service").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    dispatcher
        .events_tx
        .send(ProcessToService::StateChanged {
            id: "vm".into(),
            state: "Suspended".into(),
            trigger: "t".into(),
        })
        .unwrap();
    assert!(matches!(
        rx.recv().await.unwrap(),
        ProcessToService::StateChanged { .. }
    ));
}

/// A terminal stream that falls behind used to stop silently while its
/// connection stayed open, so the client waited on output that never came.
#[tokio::test]
async fn a_lagging_terminal_stream_ends_with_an_explicit_reason() {
    let temp = tempfile::tempdir().unwrap();
    let (dispatcher, _ctrl_rx) = Dispatcher::new(temp.path());
    let (tx, rx) = connect_as(&dispatcher, capsem_proto::handshake::STREAM_PEER_ID).await;
    tx.send(ServiceToProcess::StartTerminalStream).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Far more than the relay's broadcast capacity plus the connection's
    // outbound queue, while this client reads nothing.
    for index in 0..2_000u32 {
        dispatcher.session_terminal.publish(index.to_le_bytes());
    }
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rx.recv().await.unwrap() {
                ProcessToService::TerminalState { .. } | ProcessToService::TerminalOutput { .. } => continue,
                ProcessToService::TerminalStreamEnded { reason } => return reason,
                other => panic!("unexpected message on a terminal stream: {other:?}"),
            }
        }
    })
    .await
    .expect("a lagging stream must end explicitly");
    assert!(ended.contains("behind"), "{ended}");
}

#[tokio::test]
async fn inventory_observer_receives_browser_request_without_terminal_attachment() {
    let temp = tempfile::tempdir().unwrap();
    let (dispatcher, _) = Dispatcher::new(temp.path());
    dispatcher
        .session_browser
        .request("https://example.org/login?state=opaque")
        .unwrap();
    for _ in 0..2 {
        let (_tx, rx) = connect_as(&dispatcher, "capsem-terminal-observer").await;
        let request = loop {
            match tokio::time::timeout(Duration::from_secs(1), rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                ProcessToService::BrowserRequest { request } => break request,
                ProcessToService::TerminalSummary { .. } => {}
                other => panic!("unexpected observer message: {other:?}"),
            }
        };
        assert_eq!(request.id, 1);
        assert_eq!(request.url, "https://example.org/login?state=opaque");
    }
}

#[tokio::test]
async fn inventory_observer_receives_retained_metadata_without_a_terminal_view() {
    let temp = tempfile::tempdir().unwrap();
    let (dispatcher, _) = Dispatcher::new(temp.path());
    dispatcher.session_terminal.publish(b"\x1b]2;Agent ready\x07\x07");
    let (_tx, rx) = connect_as(&dispatcher, "capsem-terminal-observer").await;
    let ProcessToService::TerminalSummary { summary } = rx.recv().await.unwrap() else {
        panic!("expected retained metadata");
    };
    assert_eq!(summary.title, "Agent ready");
    assert_eq!(summary.bell_count, 1);
    dispatcher.session_terminal.publish(b"ordinary output");
    assert!(tokio::time::timeout(Duration::from_millis(20), rx.recv())
        .await
        .is_err());
    dispatcher.session_terminal.publish(b"\x1b]9;Review needed\x07");
    let ProcessToService::TerminalSummary { summary } = rx.recv().await.unwrap() else {
        panic!("expected notification metadata");
    };
    assert_eq!(summary.last_notification.unwrap().body, "Review needed");
}

#[tokio::test]
async fn a_stale_view_cannot_resize_or_type_into_the_active_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let (dispatcher, mut controls) = Dispatcher::new(temp.path());
    let (a, _a_rx) = connect_as(&dispatcher, capsem_proto::handshake::STREAM_PEER_ID).await;
    a.send(ServiceToProcess::StartTerminalStream).await.unwrap();
    a.send(ServiceToProcess::TerminalFocus { cols: 101, rows: 33 })
        .await
        .unwrap();
    assert!(matches!(
        controls.recv().await.unwrap(),
        ServiceToProcess::TerminalResize { cols: 101, rows: 33 }
    ));
    let (b, _b_rx) = connect_as(&dispatcher, capsem_proto::handshake::STREAM_PEER_ID).await;
    b.send(ServiceToProcess::StartTerminalStream).await.unwrap();
    b.send(ServiceToProcess::TerminalFocus { cols: 60, rows: 20 })
        .await
        .unwrap();
    assert!(matches!(
        controls.recv().await.unwrap(),
        ServiceToProcess::TerminalResize { cols: 60, rows: 20 }
    ));
    a.send(ServiceToProcess::TerminalResize { cols: 150, rows: 45 })
        .await
        .unwrap();
    a.send(ServiceToProcess::TerminalInput {
        data: b"wrong view".to_vec(),
    })
    .await
    .unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(20), controls.recv())
        .await
        .is_err());
    b.send(ServiceToProcess::TerminalInput {
        data: b"active view".to_vec(),
    })
    .await
    .unwrap();
    assert!(
        matches!(controls.recv().await.unwrap(), ServiceToProcess::TerminalInput { data } if data == b"active view")
    );
}
