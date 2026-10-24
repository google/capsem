use super::*;
use std::task::Poll;

async fn dispatch(
    sender: &impl CommandSender,
    message: ServiceToProcess,
) -> Result<(), mpsc::error::SendError<ServiceToProcess>> {
    CommandSender::send(sender, message).await
}

#[tokio::test]
async fn publication_dispatch_preserves_exact_bounded_queue_and_closed_error() {
    let (sender, mut receiver) = mpsc::channel(1);
    dispatch(&sender, ServiceToProcess::Shutdown).await.unwrap();
    let queued = dispatch(&sender, ServiceToProcess::Suspend);
    tokio::pin!(queued);
    assert!(matches!(futures::poll!(&mut queued), Poll::Pending));
    assert!(matches!(receiver.recv().await, Some(ServiceToProcess::Shutdown)));
    queued.await.unwrap();
    assert!(matches!(receiver.recv().await, Some(ServiceToProcess::Suspend)));
    drop(receiver);
    assert!(matches!(
        dispatch(&sender, ServiceToProcess::Shutdown).await.unwrap_err().0,
        ServiceToProcess::Shutdown
    ));
}

#[tokio::test]
async fn cancelling_blocked_publication_dispatch_never_enqueues_it_later() {
    let (sender, mut receiver) = mpsc::channel(1);
    dispatch(&sender, ServiceToProcess::Shutdown).await.unwrap();
    {
        let blocked = dispatch(&sender, ServiceToProcess::Suspend);
        tokio::pin!(blocked);
        assert!(matches!(futures::poll!(&mut blocked), Poll::Pending));
    }
    assert!(matches!(receiver.recv().await, Some(ServiceToProcess::Shutdown)));
    assert!(receiver.try_recv().is_err());
    dispatch(&sender, ServiceToProcess::Shutdown).await.unwrap();
    assert!(matches!(receiver.recv().await, Some(ServiceToProcess::Shutdown)));
}
