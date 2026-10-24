//! The existing bounded guest queue owns immutable final frames and replay metadata.
use capsem_proto::{framing::PreparedFrame, HostToGuest};
use std::io;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub(crate) struct Prepared {
    frame: PreparedFrame<HostToGuest>,
    ack: Option<u64>,
    publication: Option<capsem_proto::router::FlowKey>,
}

impl Prepared {
    pub(crate) fn new(message: &HostToGuest) -> io::Result<Self> {
        let ack = crate::vsock::ackable_id(message);
        let publication = match message {
            HostToGuest::ConnectPort { flow, .. } => Some(*flow),
            _ => None,
        };
        Ok(Self {
            frame: PreparedFrame::new(message, capsem_proto::MAX_FRAME_SIZE)?,
            ack,
            publication,
        })
    }

    pub(crate) fn ack(&self) -> Option<u64> {
        self.ack
    }
    pub(crate) fn publication(&self) -> Option<capsem_proto::router::FlowKey> {
        self.publication
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        self.frame.bytes()
    }
}

#[derive(Clone)]
pub(crate) struct Sender(mpsc::Sender<Prepared>);
pub(crate) struct Receiver(mpsc::Receiver<Prepared>);
pub(crate) struct Permit<'a>(mpsc::Permit<'a, Prepared>);

pub(crate) fn channel(capacity: usize) -> (Sender, Receiver) {
    let (tx, rx) = mpsc::channel(capacity);
    (Sender(tx), Receiver(rx))
}

impl Sender {
    pub(crate) async fn send(&self, message: HostToGuest) -> io::Result<()> {
        let frame = Prepared::new(&message)?;
        drop(message);
        self.send_prepared(frame).await
    }

    pub(crate) async fn send_prepared(&self, frame: Prepared) -> io::Result<()> {
        self.0.send(frame).await.map_err(|_| io::ErrorKind::BrokenPipe.into())
    }

    pub(crate) async fn reserve(&self) -> io::Result<Permit<'_>> {
        self.0
            .reserve()
            .await
            .map(Permit)
            .map_err(|_| io::ErrorKind::BrokenPipe.into())
    }
}

impl Permit<'_> {
    pub(crate) fn send(self, frame: Prepared) {
        self.0.send(frame);
    }
}

impl Receiver {
    pub(crate) async fn recv_prepared(&mut self) -> Option<Prepared> {
        self.0.recv().await
    }

    // Unit inspection decodes the actual production queue bytes, never a second
    // retained message or a fabricated successful transport response.
    #[cfg(test)]
    pub(crate) async fn recv(&mut self) -> Option<HostToGuest> {
        self.0
            .recv()
            .await
            .map(|frame| capsem_proto::decode_host_msg(&frame.bytes()[4..]).unwrap())
    }

    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Result<HostToGuest, mpsc::error::TryRecvError> {
        self.0
            .try_recv()
            .map(|frame| capsem_proto::decode_host_msg(&frame.bytes()[4..]).unwrap())
    }
}

#[cfg(test)]
mod tests;
