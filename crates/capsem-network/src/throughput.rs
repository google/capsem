//! The far end of `capsem-bench-rs throughput`, on smoltcp sockets.
//!
//! Same one-byte protocol as the bench server: the client's first byte names
//! the direction, then bytes flow until the client closes. Kept in lock-step
//! with `crates/capsem-bench/src/throughput.rs`; the wire values are pinned
//! by tests on both sides.
use crate::stack::{App, Progress};
use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp;

/// Listening slots, so the client's largest stream count fits with room for
/// the previous trial's sockets to finish closing.
pub const MAX_CONNECTIONS: usize = 32;
/// Per-direction socket buffer. Large enough that one poll round can move a
/// full link MTU without stalling on window.
pub const SOCKET_BUFFER_BYTES: usize = 256 * 1024;
/// Bytes handed to the socket per send; the client reads in 64 KiB too.
const CHUNK_BYTES: usize = 64 * 1024;
/// Echo payload of the latency direction; the client sends exactly this.
pub const ECHO_BYTES: usize = 64;
/// The port the endpoint listens on: iperf3's, so a later iperf3 lane needs
/// no new rule.
pub const THROUGHPUT_PORT: u16 = 5201;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
    Bidirectional,
    Latency,
}

impl Direction {
    pub const fn from_wire(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Upload),
            1 => Some(Self::Download),
            2 => Some(Self::Bidirectional),
            3 => Some(Self::Latency),
            _ => None,
        }
    }

    pub const fn wire(self) -> u8 {
        match self {
            Self::Upload => 0,
            Self::Download => 1,
            Self::Bidirectional => 2,
            Self::Latency => 3,
        }
    }
}

enum Phase {
    Listening,
    Header,
    Serving(Direction),
    /// Our FIN is sent; once the socket is closed the slot listens again.
    Closing,
}

struct Slot {
    handle: SocketHandle,
    phase: Phase,
    /// Latency echoes received but not yet sent back.
    pending: Vec<u8>,
}

pub struct ThroughputServer {
    port: u16,
    chunk: Vec<u8>,
    scratch: Vec<u8>,
    slots: Vec<Slot>,
    served: usize,
    rejected: usize,
}

impl ThroughputServer {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            chunk: vec![0u8; CHUNK_BYTES],
            scratch: vec![0u8; CHUNK_BYTES],
            slots: Vec::with_capacity(MAX_CONNECTIONS),
            served: 0,
            rejected: 0,
        }
    }

    /// Connections that reached the header, for tests and logs.
    pub fn served(&self) -> usize {
        self.served
    }

    /// Connections reset for an unknown direction byte.
    pub fn rejected(&self) -> usize {
        self.rejected
    }

    fn step_slot(
        slot: &mut Slot,
        socket: &mut tcp::Socket,
        port: u16,
        chunk: &[u8],
        scratch: &mut [u8],
        rejected: &mut usize,
    ) -> bool {
        match slot.phase {
            Phase::Listening => {
                // SYN-RECEIVED is "active" but cannot receive yet; moving on
                // then would read `may_recv() == false` as the client leaving
                // and reset the handshake it was still completing.
                if socket.is_active() && socket.state() != tcp::State::SynReceived {
                    slot.phase = Phase::Header;
                    return true;
                }
                if socket.state() == tcp::State::Closed {
                    // A reset handshake leaves the socket closed, not listening.
                    relisten(socket, port);
                }
                false
            }
            Phase::Header => {
                if socket.can_recv() {
                    let mut byte = [0u8; 1];
                    let read = socket.recv_slice(&mut byte).unwrap_or(0);
                    slot.phase = match Direction::from_wire(byte[0]).filter(|_| read == 1) {
                        Some(direction) => Phase::Serving(direction),
                        None => {
                            socket.abort();
                            *rejected += 1;
                            Phase::Closing
                        }
                    };
                    return true;
                }
                if !socket.may_recv() {
                    socket.abort();
                    slot.phase = Phase::Closing;
                    return true;
                }
                false
            }
            Phase::Serving(direction) => {
                let mut moved = false;
                match direction {
                    Direction::Upload | Direction::Bidirectional => {
                        while socket.can_recv() {
                            moved |= socket.recv_slice(scratch).unwrap_or(0) > 0;
                        }
                    }
                    Direction::Latency => {
                        while socket.can_recv() {
                            let read = socket.recv_slice(scratch).unwrap_or(0);
                            slot.pending.extend_from_slice(&scratch[..read]);
                            moved |= read > 0;
                        }
                        while !slot.pending.is_empty() && socket.can_send() {
                            let sent = socket.send_slice(&slot.pending).unwrap_or(0);
                            slot.pending.drain(..sent);
                            moved |= sent > 0;
                        }
                    }
                    Direction::Download => {}
                }
                if matches!(direction, Direction::Download | Direction::Bidirectional) {
                    while socket.can_send() {
                        moved |= socket.send_slice(chunk).unwrap_or(0) > 0;
                    }
                }
                // The client always closes first; answer its FIN with ours
                // once nothing is left to echo, so this side never sits in
                // TIME-WAIT and the slot is free for the next trial.
                if !socket.may_recv() && slot.pending.is_empty() {
                    socket.close();
                    slot.phase = Phase::Closing;
                    moved = true;
                }
                moved
            }
            Phase::Closing => {
                if socket.state() == tcp::State::Closed {
                    relisten(socket, port);
                    slot.phase = Phase::Listening;
                    return true;
                }
                false
            }
        }
    }
}

fn relisten(socket: &mut tcp::Socket, port: u16) {
    socket.abort();
    socket
        .listen(port)
        .expect("a closed socket can listen on a non-zero port");
}

impl App for ThroughputServer {
    fn attach(&mut self, _iface: &mut Interface, sockets: &mut SocketSet<'static>) {
        for _ in 0..MAX_CONNECTIONS {
            let mut socket = tcp::Socket::new(
                tcp::SocketBuffer::new(vec![0u8; SOCKET_BUFFER_BYTES]),
                tcp::SocketBuffer::new(vec![0u8; SOCKET_BUFFER_BYTES]),
            );
            socket.set_nagle_enabled(false);
            relisten(&mut socket, self.port);
            self.slots.push(Slot {
                handle: sockets.add(socket),
                phase: Phase::Listening,
                pending: Vec::new(),
            });
        }
    }

    fn step(&mut self, sockets: &mut SocketSet<'static>) -> Progress {
        let mut progress = Progress::Idle;
        for slot in &mut self.slots {
            let socket = sockets.get_mut::<tcp::Socket>(slot.handle);
            let was_listening = matches!(slot.phase, Phase::Listening);
            if Self::step_slot(
                slot,
                socket,
                self.port,
                &self.chunk,
                &mut self.scratch,
                &mut self.rejected,
            ) {
                progress = Progress::Changed;
                if was_listening && matches!(slot.phase, Phase::Header) {
                    self.served += 1;
                }
            }
        }
        progress
    }
}

#[cfg(test)]
mod tests;
