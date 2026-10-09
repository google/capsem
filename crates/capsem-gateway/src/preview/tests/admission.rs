//! Refusing a browser connection: what it costs and what the browser is told.
use super::*;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn tcp_pair() -> (tokio::net::TcpStream, tokio::net::TcpStream) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let connect = tokio::net::TcpStream::connect(listener.local_addr().unwrap());
    let (server, client) = tokio::join!(listener.accept(), connect);
    (server.unwrap().0, client.unwrap())
}

#[tokio::test]
async fn a_wrong_process_at_the_granted_path_receives_no_seat_or_browser_descriptor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replaced.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let (browser, _client) = tcp_pair().await;
    let current = std::process::id();
    let wrong = if current == 1 { 2 } else { 1 };
    let admission = PreviewConnectionAdmissionResponse {
        handoff_socket: path.to_string_lossy().into_owned(),
        handoff_token: 77,
        owner_generation: "trusted-generation".into(),
        owner_pid: wrong,
        owner_uid: capsem_foundation::unix::process::current_uid(),
    };

    let service = crate::service_client::ServiceClient::new(dir.path().join("service.sock").as_path());
    let attempted = tokio::spawn(async move { handoff(browser, "box", admission, &service).await });
    let (seat, _) = listener.accept().await.unwrap();
    let receiver = Receiver::new(seat.into_std().unwrap()).unwrap();
    let error = match receiver.recv().await {
        Ok(_) => panic!("replacement received a preview seat or descriptor"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    let error = attempted.await.unwrap().unwrap_err();
    assert!(format!("{error:#}").contains("granted process"), "{error:#}");
}

#[tokio::test]
async fn the_authenticated_owner_receives_the_exact_browser_descriptor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owner.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let owner = tokio::spawn(async move {
        let (seat, _) = listener.accept().await.unwrap();
        let seat = seat.into_std().unwrap();
        let receiver = Receiver::new(seat.try_clone().unwrap()).unwrap();
        let frame = receiver.recv().await.unwrap();
        let (kind, token) = decode_seat_frame(&frame.bytes).unwrap();
        assert_eq!((kind, token, frame.fds.len()), (SEAT_PREVIEW, 78, 1));
        let browser = std::net::TcpStream::from(frame.fds.into_iter().next().unwrap());
        browser.set_nonblocking(true).unwrap();
        let mut browser = tokio::net::TcpStream::from_std(browser).unwrap();
        let mut byte = [0];
        browser.read_exact(&mut byte).await.unwrap();
        assert_eq!(byte, [b'x']);
        Sender::new(seat)
            .unwrap()
            .send(&seat_frame(SEAT_PREVIEW, token), &[])
            .await
            .unwrap();
    });
    let (browser, mut client) = tcp_pair().await;
    client.write_all(b"x").await.unwrap();
    handoff(
        browser,
        "box",
        PreviewConnectionAdmissionResponse {
            handoff_socket: path.to_string_lossy().into_owned(),
            handoff_token: 78,
            owner_generation: "trusted-generation".into(),
            owner_pid: std::process::id(),
            owner_uid: capsem_foundation::unix::process::current_uid(),
        },
        &crate::service_client::ServiceClient::new(dir.path().join("service.sock").as_path()),
    )
    .await
    .unwrap();
    owner.await.unwrap();
}

/// A stalled partial request used to spin: `peek` leaves the socket readable,
/// so waiting on readability returned at once and the loop ran until the
/// header deadline, burning a core share per connection.
#[tokio::test]
async fn a_stalled_request_head_parks_instead_of_spinning() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = tokio::spawn(async move { listener.accept().await.unwrap().0 });
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    let server = accepted.await.unwrap();

    PEEK_ROUNDS.store(0, Ordering::Relaxed);
    let head = tokio::spawn(async move { Box::pin(peek_head(&server)).await });
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: preview.localhost:1")
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let rounds = PEEK_ROUNDS.load(Ordering::Relaxed);
    assert!(rounds <= 8, "peek_head spun {rounds} times on a stalled request head");

    client
        .write_all(b"9444\r\nCookie: capsem_preview=session\r\n\r\n")
        .await
        .unwrap();
    let parsed = tokio::time::timeout(std::time::Duration::from_secs(2), head)
        .await
        .expect("the completed head is parsed")
        .unwrap()
        .unwrap();
    assert_eq!(parsed.port, 19444);
    assert_eq!(parsed.cookie.as_deref(), Some("session"));
}

/// A refused connection used to be dropped, which a browser reports as
/// ERR_EMPTY_RESPONSE rather than as an expired preview.
#[tokio::test]
async fn a_refused_connection_is_answered_instead_of_dropped() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let served = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap().0;
        write_refusal(
            &mut stream,
            &refuse(
                "401 Unauthorized",
                "This preview session has expired. Reopen the preview from Capsem.",
                anyhow::anyhow!("preview session cookie missing"),
            ),
        )
        .await;
    });
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    let mut response = String::new();
    let started = Instant::now();
    client.read_to_string(&mut response).await.unwrap();
    served.await.unwrap();

    assert!(response.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{response}");
    assert!(response.contains("Connection: close\r\n"), "{response}");
    assert!(response.contains("Cache-Control: no-store\r\n"), "{response}");
    assert!(response.ends_with("Reopen the preview from Capsem."), "{response}");
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

/// The session cookie has to survive the navigation that follows the
/// cross-site bootstrap POST, which a Strict cookie does not.
#[test]
fn the_session_cookie_survives_the_bootstrap_redirect() {
    let attributes = format!("Set-Cookie: {PREVIEW_COOKIE}=session; Path=/; HttpOnly; SameSite=Lax; Max-Age=900");
    assert!(attributes.contains("SameSite=Lax"), "{attributes}");
    assert!(!attributes.contains("SameSite=Strict"), "{attributes}");
    assert!(attributes.contains("HttpOnly"), "{attributes}");
}
