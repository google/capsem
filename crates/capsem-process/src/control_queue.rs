//! Local service-command queue; prepared file writes never enter the public IPC schema.
use capsem_proto::{ipc::ServiceToProcess, HostToGuest};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub(crate) struct PreparedWrite {
    pub(crate) id: u64,
    pub(crate) observation: Arc<crate::job_store::WriteObservation>,
    pub(crate) frame: crate::guest_control::Prepared,
}

impl PreparedWrite {
    pub(crate) fn new(id: u64, path: String, data: Vec<u8>) -> std::io::Result<Self> {
        let message = HostToGuest::FileWrite {
            id,
            path,
            data,
            mode: 0o644,
        };
        let frame = crate::guest_control::Prepared::new(&message)?;
        let HostToGuest::FileWrite { path, data, .. } = message else {
            unreachable!()
        };
        let observation = Arc::new(crate::job_store::WriteObservation {
            path,
            size: data.len() as u64,
            content: crate::ipc::file_boundary::file_content_preview(&data),
        });
        Ok(Self { id, observation, frame })
    }
}

pub(crate) enum Request {
    Message(ServiceToProcess),
    Write(PreparedWrite),
}

#[derive(Clone)]
pub(crate) struct Sender(mpsc::Sender<Request>);
pub(crate) struct Receiver(mpsc::Receiver<Request>);

pub(crate) fn channel(capacity: usize) -> (Sender, Receiver) {
    let (tx, rx) = mpsc::channel(capacity);
    (Sender(tx), Receiver(rx))
}

impl Sender {
    pub(crate) async fn send(&self, message: ServiceToProcess) -> Result<(), mpsc::error::SendError<ServiceToProcess>> {
        // File writes need the IPC owner's ledger admission and prepared bytes.
        if matches!(message, ServiceToProcess::WriteFile { .. }) {
            return Err(mpsc::error::SendError(message));
        }
        self.0.send(Request::Message(message)).await.map_err(|error| {
            let Request::Message(message) = error.0 else {
                unreachable!()
            };
            mpsc::error::SendError(message)
        })
    }

    pub(crate) fn blocking_send(
        &self,
        message: ServiceToProcess,
    ) -> Result<(), mpsc::error::SendError<ServiceToProcess>> {
        if matches!(message, ServiceToProcess::WriteFile { .. }) {
            return Err(mpsc::error::SendError(message));
        }
        self.0.blocking_send(Request::Message(message)).map_err(|error| {
            let Request::Message(message) = error.0 else {
                unreachable!()
            };
            mpsc::error::SendError(message)
        })
    }

    pub(crate) async fn send_prepared(&self, write: PreparedWrite) -> std::io::Result<()> {
        self.0
            .send(Request::Write(write))
            .await
            .map_err(|_| std::io::ErrorKind::BrokenPipe.into())
    }
}

impl Receiver {
    pub(crate) async fn recv_request(&mut self) -> Option<Request> {
        self.0.recv().await
    }

    #[cfg(test)]
    pub(crate) async fn recv(&mut self) -> Option<ServiceToProcess> {
        self.0.recv().await.map(Request::into_message)
    }

    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Result<ServiceToProcess, mpsc::error::TryRecvError> {
        self.0.try_recv().map(Request::into_message)
    }
}

#[cfg(test)]
impl Request {
    fn into_message(self) -> ServiceToProcess {
        match self {
            Self::Message(message) => message,
            Self::Write(write) => {
                let HostToGuest::FileWrite { id, path, data, .. } =
                    capsem_proto::decode_host_msg(&write.frame.bytes()[4..]).unwrap()
                else {
                    unreachable!()
                };
                ServiceToProcess::WriteFile { id, path, data }
            }
        }
    }
}

impl capsem_core::container::publish::CommandSender for Sender {
    async fn send(&self, message: ServiceToProcess) -> Result<(), mpsc::error::SendError<ServiceToProcess>> {
        Sender::send(self, message).await
    }
}

#[cfg(test)]
mod tests;
