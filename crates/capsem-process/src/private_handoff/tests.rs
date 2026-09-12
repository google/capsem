use super::*;
use std::net::Ipv4Addr;
use tokio::io::AsyncWriteExt;

#[test]
fn tokens_are_sixteen_hex_digits_and_frames_carry_them_back() {
    let token = parse_token("00ff00ff00ff00ff").unwrap();
    assert_eq!(token, 0x00ff_00ff_00ff_00ff);
    for bad in ["", "ff", "zz00000000000000", "00ff00ff00ff00ff0"] {
        assert!(parse_token(bad).is_err(), "{bad}");
    }
    let frame = Frame {
        bytes: encode_token(token),
        fds: Vec::new(),
    };
    assert_eq!(decode_token(&frame).unwrap(), (FRAME_HANDOFF, token));
    let mut wrong = frame.bytes;
    wrong[1] = 1;
    assert!(
        decode_token(&Frame {
            bytes: wrong,
            fds: Vec::new()
        })
        .is_err(),
        "a router grant is not a handoff"
    );
}

#[tokio::test]
async fn a_token_is_redeemed_once_and_not_after_its_deadline() {
    let (control, _control_rx) = mpsc::channel(1);
    let handoff = PrivateHandoff::new(
        PathBuf::from("/tmp/handoff.sock"),
        Arc::new(Publisher::default()),
        control,
        PathBuf::from("/tmp/service.sock"),
        "secret".into(),
        "vm-b".into(),
        Arc::new(crate::private_link::PrivateLink::new(
            Arc::new(Publisher::default()),
            Ipv4Addr::new(10, 128, 0, 3),
        )),
    );
    let network = NetworkIdentity::parse("6ba7b810-9dad-11d1-80b4-00c04fd430c8", "team".into()).unwrap();
    let source = || source_vm("vm-a".into(), "vm-a".into(), 7);
    handoff
        .expect(
            "00000000000000aa",
            network.clone(),
            source(),
            (Ipv4Addr::new(10, 128, 0, 2), 40001).into(),
            80,
        )
        .unwrap();
    assert!(
        handoff
            .expect(
                "00000000000000aa",
                network.clone(),
                source(),
                (Ipv4Addr::new(10, 128, 0, 2), 40001).into(),
                80
            )
            .is_err(),
        "the same token cannot be expected twice"
    );
    assert!(handoff.redeem(0xaa).is_some());
    assert!(handoff.redeem(0xaa).is_none(), "redeemed once");
    handoff
        .expect(
            "00000000000000bb",
            network,
            source(),
            (Ipv4Addr::new(10, 128, 0, 2), 40001).into(),
            80,
        )
        .unwrap();
    handoff.pending.lock().unwrap().get_mut(&0xbb).unwrap().expires =
        Instant::now().checked_sub(Duration::from_millis(1)).unwrap();
    assert!(handoff.redeem(0xbb).is_none(), "expired with the setup deadline");
}

fn handoff_at(dir: &tempfile::TempDir, service_socket: PathBuf) -> Arc<PrivateHandoff> {
    let (control, _control_rx) = mpsc::channel(1);
    Arc::new(PrivateHandoff::new(
        dir.path().join("vm-b-handoff.sock"),
        Arc::new(Publisher::default()),
        control,
        service_socket,
        "secret-a".into(),
        "vm-a".into(),
        Arc::new(crate::private_link::PrivateLink::new(
            Arc::new(Publisher::default()),
            Ipv4Addr::new(10, 128, 0, 2),
        )),
    ))
}

/// A frame from a would-be source owner: the token, and the descriptors it
/// claims are the stream. Returns the far end of that stream.
async fn deliver(socket: std::os::unix::net::UnixStream, token: u64, with_stream: bool) -> tokio::net::UnixStream {
    let (stream, far) = std::os::unix::net::UnixStream::pair().unwrap();
    far.set_nonblocking(true).unwrap();
    let sender = Sender::new(socket).unwrap();
    let fds: Vec<_> = with_stream.then(|| stream.as_raw_fd()).into_iter().collect();
    sender.send(&encode_token(token), &fds).await.unwrap();
    tokio::net::UnixStream::from_std(far).unwrap()
}

async fn ends_without_a_byte(mut far: tokio::net::UnixStream) {
    let mut probe = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(2), far.read(&mut probe))
        .await
        .expect("the refused stream was kept open")
        .unwrap();
    assert_eq!(read, 0, "a refused stream ends without a byte");
}

#[tokio::test]
async fn an_unknown_token_or_a_frame_without_a_stream_is_refused_and_closed() {
    let dir = tempfile::tempdir().unwrap();
    let handoff = handoff_at(&dir, dir.path().join("service.sock"));
    let network = NetworkIdentity::parse("6ba7b810-9dad-11d1-80b4-00c04fd430c8", "team".into()).unwrap();
    handoff
        .expect(
            "00000000000000cc",
            network,
            source_vm("vm-a".into(), "vm-a".into(), 7),
            (Ipv4Addr::new(10, 128, 0, 2), 40001).into(),
            80,
        )
        .unwrap();
    for (token, with_stream, expected) in [(0xdd, true, "unknown, reused or expired"), (0xcc, false, "descriptors")] {
        let (owner_side, source_side) = std::os::unix::net::UnixStream::pair().unwrap();
        owner_side.set_nonblocking(true).unwrap();
        let taking = tokio::spawn({
            let handoff = Arc::clone(&handoff);
            async move {
                handoff
                    .take(tokio::net::UnixStream::from_std(owner_side).unwrap())
                    .await
            }
        });
        let far = deliver(source_side, token, with_stream).await;
        let refused = taking.await.unwrap().unwrap_err();
        assert!(format!("{refused:#}").contains(expected), "{refused:#}");
        ends_without_a_byte(far).await;
    }
    assert!(
        handoff.redeem(0xcc).is_none(),
        "a frame without a stream still spends the token: no second try"
    );
}

/// The service the source seat asks, answering one request as told.
async fn fake_service(
    socket: PathBuf,
    status: u16,
    answer: serde_json::Value,
) -> tokio::sync::oneshot::Receiver<serde_json::Value> {
    use axum::{routing::post, Router};
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let seen = Arc::new(Mutex::new(Some(seen_tx)));
    let app = Router::new().route(
        "/networks/private/connect",
        post(move |axum::Json(body): axum::Json<serde_json::Value>| {
            let seen = Arc::clone(&seen);
            let answer = answer.clone();
            async move {
                if let Some(seen) = seen.lock().unwrap().take() {
                    let _ = seen.send(body);
                }
                (axum::http::StatusCode::from_u16(status).unwrap(), axum::Json(answer))
            }
        }),
    );
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    seen_rx
}

fn guest_connection() -> (VsockConnection, tokio::net::UnixStream) {
    let (connection, workload) = std::os::unix::net::UnixStream::pair().unwrap();
    workload.set_nonblocking(true).unwrap();
    let conn = VsockConnection::new(connection.as_raw_fd(), 5010, Box::new(connection));
    (conn, tokio::net::UnixStream::from_std(workload).unwrap())
}

#[tokio::test]
async fn a_refused_ask_closes_the_guest_connection_before_any_byte() {
    let dir = tempfile::tempdir().unwrap();
    let service = dir.path().join("service.sock");
    let seen = fake_service(
        service.clone(),
        404,
        serde_json::json!({ "message": "no private path from vm-a to 10.128.0.9" }),
    )
    .await;
    let handoff = handoff_at(&dir, service);
    let (conn, workload) = guest_connection();
    let header = ConnectHeader {
        destination: Ipv4Addr::new(10, 128, 0, 9),
        port: 6379,
        source_port: 40001,
    };
    let refused = handoff.connect_out(conn, header, "redis-cli".into()).await.unwrap_err();
    assert!(format!("{refused:#}").contains("no private path"), "{refused:#}");
    let asked = seen.await.unwrap();
    assert_eq!(asked["source_vm"], "vm-a");
    assert_eq!(asked["owner_secret"], "secret-a");
    assert_eq!(asked["destination"], "10.128.0.9");
    assert_eq!(asked["port"], 6379);
    assert_eq!(asked["source_port"], 40001);
    assert_eq!(asked["process_name"], "redis-cli");
    assert_eq!(asked["source_generation"], handoff.publisher.generation().get());
    ends_without_a_byte(workload).await;
}

#[tokio::test]
async fn a_granted_ask_delivers_the_guest_stream_and_holds_it_until_the_destination_lets_go() {
    let dir = tempfile::tempdir().unwrap();
    let service = dir.path().join("service.sock");
    let destination_socket = dir.path().join("vm-b-handoff.sock");
    let destination = tokio::net::UnixListener::bind(&destination_socket).unwrap();
    let _seen = fake_service(
        service.clone(),
        200,
        serde_json::json!({
            "token": "00000000000000ee",
            "handoff_socket": destination_socket.to_string_lossy(),
            "destination_vm": "vm-b",
        }),
    )
    .await;
    let handoff = handoff_at(&dir, service);
    let (conn, mut workload) = guest_connection();
    let header = ConnectHeader {
        destination: Ipv4Addr::new(10, 128, 0, 9),
        port: 6379,
        source_port: 40001,
    };
    let mut source_seat = tokio::spawn(async move { handoff.connect_out(conn, header, "redis-cli".into()).await });
    // The destination owner's seat: one frame, the token, one descriptor.
    let (delivery, _) = destination.accept().await.unwrap();
    let receiver = Receiver::new(delivery.into_std().unwrap()).unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decode_token(&frame).unwrap(), (FRAME_HANDOFF, 0xee));
    assert_eq!(frame.fds.len(), 1);
    // The descriptor is the guest's stream: what the workload writes arrives
    // on it, and the source seat is still holding the connection open.
    let stream = std::os::unix::net::UnixStream::from(frame.fds.into_iter().next().unwrap());
    stream.set_nonblocking(true).unwrap();
    let mut stream = tokio::net::UnixStream::from_std(stream).unwrap();
    workload.write_all(b"PING").await.unwrap();
    let mut seen = [0u8; 4];
    stream.read_exact(&mut seen).await.unwrap();
    assert_eq!(&seen, b"PING");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut source_seat)
            .await
            .is_err(),
        "the source seat must hold the guest connection while the destination has the stream"
    );
    drop(stream);
    drop(receiver);
    tokio::time::timeout(Duration::from_secs(2), source_seat)
        .await
        .expect("the source seat did not let go when the destination closed the handoff")
        .unwrap()
        .unwrap();
    ends_without_a_byte(workload).await;
}
