use super::*;
use capsem_core::container::publish::CommandSender;

#[tokio::test]
async fn publication_uses_the_original_bounded_queue_and_original_closed_message() {
    let (sender, mut receiver) = channel(1);
    CommandSender::send(&sender, ServiceToProcess::Shutdown).await.unwrap();
    let blocked = CommandSender::send(&sender, ServiceToProcess::Suspend);
    tokio::pin!(blocked);
    assert!(matches!(futures::poll!(&mut blocked), std::task::Poll::Pending));
    assert!(matches!(
        receiver.recv_request().await,
        Some(Request::Message(ServiceToProcess::Shutdown))
    ));
    blocked.await.unwrap();
    assert!(matches!(
        receiver.recv_request().await,
        Some(Request::Message(ServiceToProcess::Suspend))
    ));
    drop(receiver);
    assert!(matches!(
        CommandSender::send(&sender, ServiceToProcess::Shutdown)
            .await
            .unwrap_err()
            .0,
        ServiceToProcess::Shutdown
    ));
}

#[tokio::test]
async fn prepared_write_and_watchdog_reuse_original_bytes_and_audit_facts() {
    let (sender, mut receiver) = channel(1);
    let write = PreparedWrite::new(99, "/workspace/exact".into(), vec![9; 8192]).unwrap();
    sender.send_prepared(write.clone()).await.unwrap();
    let Some(Request::Write(original)) = receiver.recv_request().await else {
        panic!("not a prepared write")
    };
    assert_eq!(original.frame.bytes().as_ptr(), write.frame.bytes().as_ptr());
    assert!(Arc::ptr_eq(&original.observation, &write.observation));
    sender.send_prepared(write.clone()).await.unwrap();
    let Some(Request::Write(retry)) = receiver.recv_request().await else {
        panic!("not a prepared retry")
    };
    assert_eq!(original.frame.bytes().as_ptr(), retry.frame.bytes().as_ptr());
    assert!(Arc::ptr_eq(&original.observation, &retry.observation));
    assert!(sender
        .send(ServiceToProcess::WriteFile {
            id: 100,
            path: "/not/admitted".into(),
            data: Vec::new()
        })
        .await
        .is_err());
    assert!(receiver.try_recv().is_err());
}
