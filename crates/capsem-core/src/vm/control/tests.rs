use super::*;
use std::os::fd::AsRawFd;

fn pair() -> (Connection, tokio::net::UnixStream) {
    let (socket, peer) = std::os::unix::net::UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let owner = Arc::new(VsockConnection::new(socket.as_raw_fd(), 0, Box::new(socket)));
    (
        Connection::new(owner).unwrap(),
        tokio::net::UnixStream::from_std(peer).unwrap(),
    )
}

#[tokio::test(start_paused = true)]
async fn stalled_write_expires_without_blocking_the_runtime() {
    let (mut connection, mut peer) = pair();
    let bytes = vec![7; 4 * 1024 * 1024];
    let result = tokio::time::timeout(IO_DEADLINE * 2, connection.write(&bytes)).await;
    assert!(result.is_ok(), "control writer has no deadline");
    assert!(result.unwrap().is_err());
    let mut received = Vec::new();
    tokio::time::timeout(IO_DEADLINE, peer.read_to_end(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert!(received.len() < bytes.len());
    connection.close().await;
    assert!(connection.reader.is_empty());
}

#[tokio::test]
async fn cancelling_receive_keeps_partial_frame_and_close_joins_reader() {
    let (mut connection, mut peer) = pair();
    let frame = capsem_proto::encode_guest_msg(&GuestToHost::Pong).unwrap();
    peer.write_all(&frame[..2]).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(10), connection.recv())
        .await
        .is_err());
    peer.write_all(&frame[2..]).await.unwrap();
    assert!(matches!(connection.recv().await.unwrap().unwrap(), GuestToHost::Pong));
    connection.close().await;
    assert!(connection.reader.is_empty());
    assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test(start_paused = true)]
async fn idle_is_allowed_but_an_incomplete_frame_expires() {
    let (mut connection, mut peer) = pair();
    assert!(tokio::time::timeout(IO_DEADLINE * 2, connection.recv()).await.is_err());
    peer.write_all(&[0]).await.unwrap();
    let error = connection.recv().await.unwrap().unwrap_err();
    assert!(error.to_string().contains("timed out"));
    connection.close().await;
}

#[tokio::test]
async fn oversized_frame_is_drained_before_the_next_message() {
    let (mut connection, mut peer) = pair();
    let size = MAX_FRAME_SIZE + 1;
    let producer = async {
        peer.write_all(&size.to_be_bytes()).await.unwrap();
        peer.write_all(&vec![0; size as usize]).await.unwrap();
        peer.write_all(&capsem_proto::encode_guest_msg(&GuestToHost::Pong).unwrap())
            .await
            .unwrap();
    };
    let receiver = async {
        assert!(matches!(connection.recv().await.unwrap().unwrap(), GuestToHost::Pong));
    };
    tokio::time::timeout(IO_DEADLINE, async {
        tokio::join!(producer, receiver);
    })
    .await
    .unwrap();
    connection.close().await;
}

#[tokio::test]
async fn drop_wakes_peer_even_while_reader_waits_on_a_partial_frame() {
    let (connection, mut peer) = pair();
    peer.write_all(&[0, 0]).await.unwrap();
    drop(connection);
    // Linux resets an abandoned Unix stream with unread bytes; Darwin sends
    // EOF. This is abrupt control teardown, not the TCP FIN/RST contract.
    let disconnected = tokio::time::timeout(IO_DEADLINE, peer.read(&mut [0])).await.unwrap();
    match disconnected {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("control peer remained live or failed unexpectedly: {other:?}"),
    }
}
