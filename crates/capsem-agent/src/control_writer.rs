//! The serialized control-channel writer shared by every host connection.
//!
//! One channel outlives every connection; each connection's writer takes its
//! turn on the shared receiver under a lease flag, replays unacked responses,
//! and hands back anything it could not deliver. See `CtrlSender`.

use std::os::unix::io::RawFd;
use std::thread;

use capsem_proto::{framing::PreparedFrame, GuestToHost};
use nix::libc;

use crate::ackable_response_id;
use crate::shutdown::HostShutdown;
use crate::vsock_io::write_all_fd;

mod network;

#[cfg(test)]
mod tests;

pub(crate) type PendingResponses = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, Queued>>>;

pub(crate) type SharedCtrlReceiver = std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<Queued>>>;

/// Queued network reports and browser requests retain their own credit until
/// delivery. Reconnects cannot free slots ahead of queue drain.
#[derive(Debug, Clone)]
pub(crate) struct Queued {
    pub(crate) frame: Result<PreparedFrame<GuestToHost>, std::sync::Arc<Rejected>>,
    id: Option<u64>,
    shutdown: bool,
    credit: Option<std::sync::Arc<tokio::sync::OwnedSemaphorePermit>>,
    acknowledged: Option<std::sync::Arc<tokio::sync::Notify>>,
}

#[derive(Debug)]
pub(crate) struct Rejected;

impl Queued {
    pub(crate) fn prepare(message: &GuestToHost) -> std::io::Result<Self> {
        Ok(Self {
            frame: Ok(PreparedFrame::new(message, capsem_proto::MAX_FRAME_SIZE)?),
            id: ackable_response_id(message),
            shutdown: matches!(message, GuestToHost::ShutdownComplete),
            credit: None,
            acknowledged: None,
        })
    }

    // Preserve the writer's existing nonfatal drop and real channel-close
    // result without retaining an unframeable body in either queue or replay.
    fn rejected(message: &GuestToHost) -> Self {
        Self {
            frame: Err(std::sync::Arc::new(Rejected)),
            id: ackable_response_id(message),
            shutdown: false,
            credit: None,
            acknowledged: None,
        }
    }

    fn bytes_or_drop<'a>(&'a self, pending: &PendingResponses) -> Option<&'a [u8]> {
        match &self.frame {
            Ok(frame) => Some(frame.bytes()),
            Err(rejected) => {
                if let Some(id) = self.id {
                    let mut pending = pending.lock().unwrap_or_else(|error| error.into_inner());
                    if pending
                        .get(&id)
                        .and_then(|queued| queued.frame.as_ref().err())
                        .is_some_and(|current| std::sync::Arc::ptr_eq(current, rejected))
                    {
                        pending.remove(&id);
                    }
                    drop(pending);
                }
                None
            }
        }
    }

    fn admit(self, sender: &CtrlSender) -> Result<(), std::sync::mpsc::SendError<Self>> {
        if let Some(id) = self.id.filter(|_| self.frame.is_ok()) {
            let mut pending = sender.pending.lock().unwrap_or_else(|e| e.into_inner());
            if pending.len() >= 4096 {
                pending.clear();
            }
            pending.insert(id, self.clone());
            drop(pending);
        }
        sender.tx.send(self)
    }

    #[cfg(test)]
    pub(crate) fn message(&self) -> GuestToHost {
        capsem_proto::decode_guest_msg(&self.frame.as_ref().unwrap().bytes()[4..]).unwrap()
    }
}

/// Producer side of the serialized control-channel writer.
///
/// One channel outlives every connection. The exec threads hold clones and
/// keep running across a reconnect, so an ExecDone that finishes after the
/// old connection died must reach the new connection's writer -- with a
/// per-connection channel it went to a dropped receiver and the host waited
/// for the exit code forever. Each connection's writer takes its turn on the
/// shared receiver.
///
/// Ackable responses are parked in `pending` here, at send time, so nothing
/// that enters the channel can be lost with it; the host's AckReply removes
/// them and every fresh connection replays what is left.
#[derive(Clone)]
pub(crate) struct CtrlSender {
    browser_credit: std::sync::Arc<tokio::sync::Semaphore>,
    tx: std::sync::mpsc::Sender<Queued>,
    pub(crate) pending: PendingResponses,
    pub(crate) network: std::sync::Arc<network::Reports>,
    /// The host-initiated shutdown handshake; see `HostShutdown`.
    pub(crate) shutdown: std::sync::Arc<HostShutdown>,
}

impl CtrlSender {
    pub(crate) fn new(pending: PendingResponses) -> (Self, SharedCtrlReceiver) {
        let (tx, rx) = std::sync::mpsc::channel();
        let sender = Self {
            browser_credit: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            tx,
            pending,
            network: std::sync::Arc::new(network::Reports::default()),
            shutdown: std::sync::Arc::new(HostShutdown::default()),
        };
        (sender, std::sync::Arc::new(std::sync::Mutex::new(rx)))
    }

    pub(crate) fn send(&self, mut msg: GuestToHost) -> Result<(), std::sync::mpsc::SendError<GuestToHost>> {
        if matches!(msg, GuestToHost::PortClosed { .. } | GuestToHost::OpenUrl { .. }) {
            return Err(std::sync::mpsc::SendError(msg));
        }
        let queued = match Queued::prepare(&msg) {
            Ok(queued) => queued,
            Err(error) => {
                if let GuestToHost::FileContent { id, path, .. } = &msg {
                    msg = GuestToHost::Error {
                        id: *id,
                        message: format!(
                            "failed to read {path}: file too large for one control frame ({} bytes)",
                            capsem_proto::MAX_FRAME_SIZE
                        ),
                    };
                    Queued::prepare(&msg).unwrap_or_else(|_| Queued::rejected(&msg))
                } else {
                    eprintln!("[capsem-agent] refusing unframeable control message: {error}");
                    Queued::rejected(&msg)
                }
            }
        };
        queued.admit(self).map_err(|_| std::sync::mpsc::SendError(msg))
    }

    fn pending_snapshot(&self) -> Vec<Queued> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    pub(crate) fn send_browser(&self, url: &str) -> std::io::Result<()> {
        let url = capsem_proto::browser::browser_url(url).map_err(std::io::Error::other)?;
        let credit = self
            .browser_credit
            .clone()
            .try_acquire_owned()
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::WouldBlock))?;
        let mut queued = Queued::prepare(&GuestToHost::OpenUrl {
            url: capsem_proto::browser::BrowserUrl(url),
        })?;
        queued.credit = Some(std::sync::Arc::new(credit));
        self.tx
            .send(queued)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }

    pub(crate) fn send_closed(
        &self,
        flow: capsem_proto::router::FlowKey,
        report: capsem_proto::router::CloseReport,
        credit: std::sync::Arc<tokio::sync::OwnedSemaphorePermit>,
    ) -> std::io::Result<std::sync::Arc<tokio::sync::Notify>> {
        let (acknowledged, sent) = {
            let queued = self.network.park(flow, report, credit)?;
            (queued.acknowledged.as_ref().unwrap().clone(), self.tx.send(queued))
        };
        if sent.is_err() {
            self.network.acknowledge(flow);
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        }
        Ok(acknowledged)
    }
}

/// State that outlives any one host connection and is handed to every
/// `run_bridge`: exec bookkeeping and the serialized control writer channel.
#[derive(Clone)]
pub(crate) struct BridgeShared {
    pub(crate) exec_inflight:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, std::sync::Arc<crate::ExecCancellation>>>>,
    pub(crate) exec_done: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, i32>>>,
    pub(crate) ctrl_sender: CtrlSender,
    pub(crate) ctrl_rx: SharedCtrlReceiver,
}

/// How long a connection's writer waits on the shared receiver before
/// re-checking whether its connection is still alive.
pub(crate) const CTRL_WRITER_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// One connection's turn at the control channel writer.
///
/// Replays every unacked response first, then drains the shared receiver
/// until the connection dies. On a failed write both vsock fds are shut down
/// so bridge_loop and control_loop wake from their polls and the outer
/// reconnect logic runs. The message that failed is requeued for the next
/// connection's writer, whose replay snapshot may already have been taken;
/// the host dedups by id, so a copy that did land is harmless.
pub(crate) fn control_writer_loop(
    rx: &SharedCtrlReceiver,
    sender: &CtrlSender,
    control_fd: RawFd,
    terminal_fd: RawFd,
    alive: &std::sync::atomic::AtomicBool,
) {
    use std::sync::atomic::Ordering;
    let mark_dead = || {
        alive.store(false, Ordering::SeqCst);
        unsafe {
            libc::shutdown(control_fd, libc::SHUT_RDWR);
            libc::shutdown(terminal_fd, libc::SHUT_RDWR);
        }
    };

    // Replay every entry still in pending_responses on the fresh control_fd.
    // These are ackable responses (ExecDone / FileOpDone / FileContent /
    // Error) whose host-side AckReply never arrived -- typically because the
    // previous control conn went silent (Apple VZ post-restoreState pattern:
    // write returned success, bytes never propagated). The host's
    // handle_guest_msg is idempotent for already-handled ids.
    let mut replayed = false;
    for queued in sender.pending_snapshot() {
        if !replayed {
            eprintln!("[capsem-agent] control writer: replaying pending unacked responses");
            replayed = true;
        }
        let Some(frame) = queued.bytes_or_drop(&sender.pending) else {
            continue;
        };
        if write_all_fd(control_fd, frame).is_err() {
            mark_dead();
            return;
        }
    }

    // At most one report per reserved network credit. A replay never appends
    // another queue entry; its snapshot retains the original credit.
    for queued in sender.network.snapshot() {
        let Some(frame) = queued.bytes_or_drop(&sender.pending) else {
            continue;
        };
        if write_all_fd(control_fd, frame).is_err() {
            mark_dead();
            return;
        }
    }

    while alive.load(Ordering::SeqCst) {
        if sender.network.take_failure() {
            mark_dead();
            return;
        }
        let queued = match rx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recv_timeout(CTRL_WRITER_POLL)
        {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let Some(frame) = queued.bytes_or_drop(&sender.pending) else {
            continue;
        };
        if write_all_fd(control_fd, frame).is_err() {
            mark_dead();
            if (queued.id.is_some() || queued.credit.is_some()) && sender.tx.send(queued).is_err() {
                tracing::error!("control replay queue closed after transport failure");
            }
            return;
        }
        if queued.shutdown {
            sender.shutdown.mark_reported();
        }
        drop(queued);
    }
}

/// How often the heartbeat checks its connection lease between probes.
pub(crate) const HEARTBEAT_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// Probe interval: six polls.
pub(crate) const HEARTBEAT_POLLS_PER_PROBE: u32 = 6;

/// Send a Pong every ~3s while `alive` holds, and return promptly once it
/// drops so `run_bridge` can join this thread before its fds are closed.
pub(crate) fn heartbeat_loop(sender: &CtrlSender, alive: &std::sync::atomic::AtomicBool) {
    loop {
        for _ in 0..HEARTBEAT_POLLS_PER_PROBE {
            thread::sleep(HEARTBEAT_POLL);
            if !alive.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
        }
        let _ = sender.send(GuestToHost::Pong);
    }
}
