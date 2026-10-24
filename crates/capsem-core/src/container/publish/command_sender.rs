//! Publication dispatch borrows the VM owner's existing bounded control queue.
use capsem_proto::ipc::ServiceToProcess;
use std::future::Future;
use tokio::sync::mpsc;

/// No forwarding channel/task: implementations await the original queue's
/// admission and return its original closed-queue message on failure.
pub trait CommandSender: Clone + Send + Sync + 'static {
    fn send(
        &self,
        message: ServiceToProcess,
    ) -> impl Future<Output = Result<(), mpsc::error::SendError<ServiceToProcess>>> + Send;
}

impl CommandSender for mpsc::Sender<ServiceToProcess> {
    async fn send(&self, message: ServiceToProcess) -> Result<(), mpsc::error::SendError<ServiceToProcess>> {
        mpsc::Sender::send(self, message).await
    }
}
