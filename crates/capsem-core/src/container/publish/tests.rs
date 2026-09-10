use super::*;

#[tokio::test]
async fn compromised_router_cannot_duplicate_connections_or_leave_pending_state() {
    let owner = Arc::new(Publisher::default());
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
    let (control, mut requests) = mpsc::channel(4);
    let broker = tokio::spawn(broker::serve(
        owner.clone(),
        6379,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
    ));
    let mut child = UnixStream::from_std(child).unwrap();
    Event::Open(1).write(&mut child).await.unwrap();
    assert!(matches!(
        requests.recv().await.unwrap(),
        ServiceToProcess::ConnectPort { port: 6379, .. }
    ));
    Event::Open(1).write(&mut child).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(1), broker)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    tokio::time::timeout(Duration::from_secs(1), async {
        while !owner.pending.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn compromised_router_cannot_allocate_from_an_attacker_controlled_frame_length() {
    use tokio::io::AsyncWriteExt;
    let owner = Arc::new(Publisher::default());
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
    let (control, _requests) = mpsc::channel(4);
    let broker = tokio::spawn(broker::serve(
        owner,
        6379,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
    ));
    let mut child = UnixStream::from_std(child).unwrap();
    child.write_all(&[255; 9]).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(1), broker)
        .await
        .unwrap()
        .unwrap()
        .is_err());
}
