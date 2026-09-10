use super::*;
use capsem_foundation::unix::router_channel::{Receiver, Sender};
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};

fn stream_pair() -> (StdUnixStream, UnixStream) {
    let (owned, peer) = StdUnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    (owned, UnixStream::from_std(peer).unwrap())
}

#[tokio::test]
async fn connected_pair_preserves_binary_half_close_and_concurrency() {
    let (parent, child) = StdUnixStream::pair().unwrap();
    let sender = Sender::new(parent.try_clone().unwrap()).unwrap();
    let receiver = Receiver::new(child.try_clone().unwrap()).unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let mut events = UnixStream::from_std(parent).unwrap();
    let router = tokio::spawn(relay(receiver, UnixStream::from_std(child).unwrap()));
    assert_eq!(Event::read(&mut events).await.unwrap(), Event::Ready);
    let mut peers = tokio::task::JoinSet::new();
    let mut retained = HashMap::new();
    for id in 1..=32 {
        let (source, mut client) = stream_pair();
        let (destination, mut server) = stream_pair();
        send_grant(
            &sender,
            Grant::Connected {
                id,
                source: source.as_fd(),
                destination: destination.as_fd(),
            },
        )
        .await
        .unwrap();
        retained.insert(id, (source, destination));
        peers.spawn(async move {
            let payload = vec![id as u8; 32 * 1024];
            let echo = tokio::spawn(async move {
                let mut bytes = Vec::new();
                server.read_to_end(&mut bytes).await.unwrap();
                server.write_all(&bytes).await.unwrap();
                server.shutdown().await.unwrap();
            });
            client.write_all(&payload).await.unwrap();
            client.shutdown().await.unwrap();
            let mut bytes = Vec::new();
            client.read_to_end(&mut bytes).await.unwrap();
            assert_eq!(bytes, payload);
            echo.await.unwrap();
        });
    }
    let mut accepted = 0;
    let mut closed = 0;
    while closed < 32 {
        match timeout(Duration::from_secs(5), Event::read(&mut events))
            .await
            .unwrap()
            .unwrap()
        {
            Event::Accepted(id) => {
                retained.remove(&id).expect("acknowledged unknown handoff");
                accepted += 1;
            }
            Event::Closed(_) => closed += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(accepted, 32);
    while let Some(result) = peers.join_next().await {
        result.unwrap();
    }
    drop(sender);
    drop(events);
    assert!(timeout(Duration::from_secs(2), router).await.unwrap().unwrap().is_err());
}

#[tokio::test]
async fn hostile_or_wrong_version_event_is_rejected_after_fixed_frame() {
    for frame in [[255; 10], [0; 10]] {
        let (mut writer, mut reader) = tokio::io::duplex(10);
        writer.write_all(&frame).await.unwrap();
        assert_eq!(
            Event::read(&mut reader).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}

#[tokio::test]
async fn grant_rejects_missing_or_excess_descriptors_and_versions() {
    use capsem_foundation::unix::router_channel::Frame;
    let (source, _) = StdUnixStream::pair().unwrap();
    for count in [0, 1, 3] {
        let fds = (0..count)
            .map(|_| capsem_foundation::unix::fd::duplicate(source.as_fd()).unwrap())
            .collect();
        assert!(Grant::decode(Frame {
            bytes: encode(1, 1),
            fds
        })
        .is_err());
    }
    assert!(Grant::decode(Frame {
        bytes: [0; 10],
        fds: vec![]
    })
    .is_err());
}

#[tokio::test]
async fn startup_failure_is_distinct_from_ready() {
    let (mut writer, mut reader) = tokio::io::duplex(10);
    Event::ConfinementFailed.write(&mut writer).await.unwrap();
    assert_eq!(Event::read(&mut reader).await.unwrap(), Event::ConfinementFailed);
}
