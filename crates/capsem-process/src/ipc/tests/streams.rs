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
        dispatcher.term_relay.publish(index.to_le_bytes().to_vec());
    }
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rx.recv().await.unwrap() {
                ProcessToService::TerminalOutput { .. } => continue,
                ProcessToService::TerminalStreamEnded { reason } => return reason,
                other => panic!("unexpected message on a terminal stream: {other:?}"),
            }
        }
    })
    .await
    .expect("a lagging stream must end explicitly");
    assert!(ended.contains("behind"), "{ended}");
}
