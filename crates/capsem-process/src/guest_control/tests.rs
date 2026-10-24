use super::*;

#[tokio::test]
async fn oversized_original_never_enters_queue_and_next_operation_is_healthy() {
    let (sender, mut receiver) = channel(1);
    assert!(sender
        .send(HostToGuest::FileWrite {
            id: 4,
            path: "/workspace/large".into(),
            data: vec![5; capsem_proto::MAX_FRAME_SIZE as usize],
            mode: 0o644
        })
        .await
        .is_err());
    assert!(receiver.try_recv().is_err());
    sender.send(HostToGuest::Ping { epoch_secs: 7 }).await.unwrap();
    let frame = receiver.recv_prepared().await.unwrap();
    assert!(frame.ack().is_none());
    assert_eq!(
        frame.bytes(),
        capsem_proto::encode_host_msg(&HostToGuest::Ping { epoch_secs: 7 }).unwrap()
    );
}

#[tokio::test]
async fn prepared_queue_keeps_backpressure_and_replay_shares_final_bytes() {
    let (sender, mut receiver) = channel(1);
    let message = HostToGuest::Exec {
        id: 17,
        command: "original".into(),
    };
    let frame = Prepared::new(&message).unwrap();
    let replay = frame.clone();
    sender.send_prepared(frame).await.unwrap();
    let next = sender.send(HostToGuest::Ping { epoch_secs: 1 });
    tokio::pin!(next);
    assert!(matches!(futures::poll!(&mut next), std::task::Poll::Pending));
    let received = receiver.recv_prepared().await.unwrap();
    assert_eq!(received.bytes().as_ptr(), replay.bytes().as_ptr());
    assert_eq!(received.ack(), Some(17));
    next.await.unwrap();
    drop(receiver);
    assert!(sender.send(HostToGuest::Ping { epoch_secs: 2 }).await.is_err());
}
