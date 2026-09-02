//! The serialized control-channel writer shared by every host connection.
//!
//! One channel outlives every connection; each connection's writer takes its
//! turn on the shared receiver under a lease flag, replays unacked responses,
//! and hands back anything it could not deliver. See `CtrlSender`.

use std::os::unix::io::RawFd;
use std::thread;

use capsem_proto::{encode_guest_msg, GuestToHost};
use nix::libc;

use crate::ackable_response_id;
use crate::vsock_io::write_all_fd;

/// Frame `msg` for the control channel. A message that cannot be framed (over
/// `MAX_FRAME_SIZE`) is logged, evicted from the replay map, and dropped:
/// the host would never decode or ack it, and replaying it on every
/// reconnect is how one oversized response wedged the channel for good.
pub(crate) fn frame_or_drop(msg: &GuestToHost, pending: &PendingResponses) -> Option<Vec<u8>> {
    match encode_guest_msg(msg) {
        Ok(frame) => Some(frame),
        Err(e) => {
            eprintln!("[capsem-agent] control writer: dropping unframeable message: {e}");
            if let Some(id) = ackable_response_id(msg) {
                pending.lock().unwrap().remove(&id);
            }
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) type PendingResponses = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, GuestToHost>>>;

pub(crate) type SharedCtrlReceiver = std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<GuestToHost>>>;

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
    pub(crate) tx: std::sync::mpsc::Sender<GuestToHost>,
    pub(crate) pending: PendingResponses,
}

impl CtrlSender {
    pub(crate) fn new(pending: PendingResponses) -> (Self, SharedCtrlReceiver) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { tx, pending }, std::sync::Arc::new(std::sync::Mutex::new(rx)))
    }

    pub(crate) fn send(&self, msg: GuestToHost) -> Result<(), std::sync::mpsc::SendError<GuestToHost>> {
        if let Some(id) = ackable_response_id(&msg) {
            let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            // Bound the map at 4096 to match the exec_done cap -- under
            // transport pathology it could otherwise grow without limit.
            if p.len() >= 4096 {
                p.clear();
            }
            p.insert(id, msg.clone());
        }
        self.tx.send(msg)
    }
}

/// State that outlives any one host connection and is handed to every
/// `run_bridge`: exec bookkeeping and the serialized control writer channel.
#[derive(Clone)]
pub(crate) struct BridgeShared {
    pub(crate) exec_inflight: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<u64>>>,
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
    let snap: Vec<GuestToHost> = sender
        .pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect();
    if !snap.is_empty() {
        eprintln!(
            "[capsem-agent] control writer: replaying {} pending unacked responses",
            snap.len()
        );
        for msg in &snap {
            let Some(frame) = frame_or_drop(msg, &sender.pending) else {
                continue;
            };
            if write_all_fd(control_fd, &frame).is_err() {
                mark_dead();
                return;
            }
        }
    }

    while alive.load(Ordering::SeqCst) {
        let received = rx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recv_timeout(CTRL_WRITER_POLL);
        let msg = match received {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        };
        // Frame first: a message that cannot be framed is a bug in the
        // producer, not a transport loss, and must not tear the connection
        // down.
        let Some(frame) = frame_or_drop(&msg, &sender.pending) else {
            continue;
        };
        if write_all_fd(control_fd, &frame).is_err() {
            mark_dead();
            if ackable_response_id(&msg).is_some() {
                let _ = sender.tx.send(msg);
            }
            return;
        }
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
