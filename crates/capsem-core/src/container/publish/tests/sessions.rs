//! Finding 11 (google/capsem#222): a preview session's lifetime and revocation
//! must bound the connections it admitted, not only the admission itself.
use super::*;
use crate::security_engine::network::NetworkReason;

fn session(preview: &PreviewState) -> String {
    preview.exchange(&preview.create_session().unwrap()).unwrap()
}

fn lease(preview: &PreviewState, session: &str) -> SessionLease {
    let handoff = preview
        .admit(session, capsem_proto::PreviewAdmissionKind::WebsocketUpgrade)
        .unwrap();
    let (kind, lease) = preview.redeem(handoff).unwrap();
    assert_eq!(kind, capsem_proto::PreviewAdmissionKind::WebsocketUpgrade);
    lease
}

#[test]
fn a_redeemed_handoff_carries_its_session_lease() {
    let (incoming, _requests) = mpsc::channel(1);
    let preview = PreviewState::new(incoming);
    let lease = lease(&preview, &session(&preview));
    assert_eq!(
        lease.ended(std::time::Instant::now()),
        None,
        "a live session's flow keeps running"
    );
}

#[test]
fn a_session_lease_ends_when_the_session_expires() {
    let (incoming, _requests) = mpsc::channel(1);
    let preview = PreviewState::new(incoming);
    let lease = lease(&preview, &session(&preview));
    let expiry = std::time::Instant::now() + PREVIEW_SESSION_LIFETIME;
    assert_eq!(
        lease.ended(expiry + Duration::from_secs(1)),
        Some(NetworkReason::SessionExpired),
        "a WebSocket admitted before expiry must not outlive the session"
    );
}

#[test]
fn revoking_every_session_ends_their_flows_and_keeps_the_exposure() {
    let (incoming, _requests) = mpsc::channel(1);
    let preview = PreviewState::new(incoming);
    let flows = [lease(&preview, &session(&preview)), lease(&preview, &session(&preview))];

    let revoked = session(&preview);
    let flows = [flows, [lease(&preview, &revoked), lease(&preview, &revoked)]].concat();
    assert_eq!(preview.revoke_all(), 3);
    let now = std::time::Instant::now();
    for flow in &flows {
        assert_eq!(flow.ended(now), Some(NetworkReason::SessionRevoked));
    }
    assert!(
        preview
            .admit(&revoked, capsem_proto::PreviewAdmissionKind::Request)
            .is_err(),
        "a revoked session admits nothing new"
    );
    // A legitimate user bootstraps again; the exposure itself was not torn down.
    let fresh = lease(&preview, &session(&preview));
    assert_eq!(fresh.ended(now), None);
}

const ALLOW_REDIS: &str = "[profiles.rules.allow_redis]\nname = \"allow_redis\"\naction = \"allow\"\n\
                           match = 'network.destination.port == \"6379\"'";

/// Through the real broker and a fake router child: once the session is
/// revoked, the admitted flow is aborted and the ledger says why.
#[tokio::test]
async fn an_admitted_preview_flow_is_aborted_when_its_sessions_are_revoked() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, path) = super::security::lifecycle_engine(&dir, ALLOW_REDIS);
    let owner = Arc::new(Publisher::default().with_security("vm-id".into(), "redis".into(), engine.clone()));
    owner.control_ready().unwrap();
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
    child.set_nonblocking(true).unwrap();
    let mut events = UnixStream::from_std(child.try_clone().unwrap()).unwrap();
    let receiver = capsem_foundation::unix::router_channel::Receiver::new(child).unwrap();
    let router = Arc::new(companion::Router::new(0, sender, CancellationToken::new()));
    let monitor = router.clone();
    let reader = tokio::spawn(async move { monitor.read_events(UnixStream::from_std(parent).unwrap()).await });
    let (control, mut requests) = mpsc::channel(4);
    let (feed, incoming) = mpsc::channel(4);
    let broker = tokio::spawn(broker::serve(
        owner.clone(),
        incoming,
        control,
        router.clone(),
        CancellationToken::new(),
    ));

    let (preview_feed, _unused) = mpsc::channel(1);
    let preview = PreviewState::new(preview_feed);
    let session_lease = lease(&preview, &session(&preview));
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let _browser = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (accepted, peer) = listener.accept().unwrap();
    let audit = crate::container::publish::security::AuditFlow::preview(
        owner.security.clone().unwrap(),
        uuid::Uuid::new_v4(),
        accepted.local_addr().unwrap(),
        peer,
        6379,
        capsem_proto::PreviewAdmissionKind::WebsocketUpgrade,
    );
    feed.send(Incoming {
        source: Source(accepted),
        audit,
        port: 6379,
        target: capsem_proto::PublicationTarget::Container,
        preview: Some(capsem_proto::PreviewAdmissionKind::WebsocketUpgrade),
        session: Some(session_lease),
    })
    .await
    .unwrap();

    let ServiceToProcess::ConnectPort { flow, .. } = requests.recv().await.unwrap() else {
        panic!("missing guest setup");
    };
    let (connection, guest) = StdUnixStream::pair().unwrap();
    guest.set_nonblocking(true).unwrap();
    let mut guest = UnixStream::from_std(guest).unwrap();
    guest.write_all(&flow.data_header(true)).await.unwrap();
    owner.accept(VsockConnection::new(connection.as_raw_fd(), 0, Box::new(connection)));
    let granted = Grant::decode(receiver.recv().await.unwrap()).unwrap();
    assert!(matches!(granted, Grant::Preview { id: 1, .. }));
    Event::Accepted(1).write(&mut events).await.unwrap();

    assert_eq!(preview.revoke_all(), 1);
    let abort = Grant::decode(
        tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("a revoked session's flow must be aborted")
            .unwrap(),
    )
    .unwrap();
    assert!(matches!(abort, Grant::Abort { id: 1 }), "expected the flow's abort");
    Event::Closed(
        1,
        capsem_proto::router::CloseReport {
            reason: capsem_proto::router::CloseReason::Reset,
            from_source: 0,
            to_source: 0,
        },
    )
    .write(&mut events)
    .await
    .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let rows = loop {
        engine.db.flush_checked().await.unwrap();
        let rows = capsem_logger::DbReader::open(&path)
            .unwrap()
            .query_raw_with_params(
                "SELECT event_json FROM transport_events WHERE event_type = 'network.close'",
                &[],
            )
            .unwrap();
        if rows.contains("\"reason\"") || tokio::time::Instant::now() >= deadline {
            break rows;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    // Rows arrive as JSON inside a JSON string, so the inner quotes are escaped.
    assert!(
        rows.contains(r#"\"reason\":\"session_revoked\""#),
        "the ledger records why the flow ended: {rows}"
    );

    router.closed.cancel();
    drop((broker, reader, granted, abort, guest));
}
