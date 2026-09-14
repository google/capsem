use super::*;
use capsem_foundation::unix::router_channel::Receiver;

#[tokio::test]
async fn late_ack_after_removal_does_not_interrupt_another_publication() {
    let report = capsem_router::CloseReport {
        reason: capsem_router::CloseReason::Cancelled,
        from_source: 7,
        to_source: 5,
    };
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let receiver = Receiver::new(child.try_clone().unwrap()).unwrap();
    let mut events = UnixStream::from_std(child).unwrap();
    let router = Arc::new(Router::new(
        0,
        Sender::new(parent.try_clone().unwrap()).unwrap(),
        CancellationToken::new(),
    ));
    let monitor = router.clone();
    let reader = tokio::spawn(async move { monitor.read_events(UnixStream::from_std(parent).unwrap()).await });
    let (first, mut first_events) = mpsc::channel(4);
    let (second, mut second_events) = mpsc::channel(4);
    let (source, _client) = StdUnixStream::pair().unwrap();
    let (destination, _server) = StdUnixStream::pair().unwrap();
    let first = router.grant(source.as_fd(), destination.as_fd(), first).await.unwrap();
    let _first_pair = receiver.recv().await.unwrap();
    let second = router.grant(source.as_fd(), destination.as_fd(), second).await.unwrap();
    let _second_pair = receiver.recv().await.unwrap();
    router.abort(first).await.unwrap();
    assert!(matches!(Grant::decode(receiver.recv().await.unwrap()).unwrap(), Grant::Abort { id } if id == first));
    Event::Accepted(first).write(&mut events).await.unwrap();
    Event::Closed(first, report).write(&mut events).await.unwrap();
    Event::Accepted(second).write(&mut events).await.unwrap();
    assert_eq!(second_events.recv().await.unwrap(), Event::Accepted(second));
    assert!(first_events.recv().await.is_none());
    Event::Closed(second, report).write(&mut events).await.unwrap();
    assert_eq!(second_events.recv().await.unwrap(), Event::Closed(second, report));
    assert!(router.observers.lock().unwrap().is_empty());
    router.closed.cancel();
    reader.await.unwrap().unwrap();
}
