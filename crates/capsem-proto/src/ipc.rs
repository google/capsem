use serde::{Deserialize, Serialize};
use std::os::unix::io::RawFd;

/// Messages sent from capsem-service to capsem-process over the per-VM Unix Domain Socket.
#[derive(Serialize, Deserialize, Debug)]
pub enum ServiceToProcess {
    /// Ping the process to check if it's alive and responsive.
    Ping,
    /// Send input bytes to the guest PTY.
    TerminalInput { data: Vec<u8> },
    /// Resize the guest PTY.
    TerminalResize { cols: u16, rows: u16 },
    /// Request the process to gracefully shut down the VM.
    Shutdown,
}

/// Messages sent from capsem-process back to capsem-service over the per-VM UDS.
#[derive(Serialize, Deserialize, Debug)]
pub enum ProcessToService {
    /// Response to Ping.
    Pong,
    /// Output bytes from the guest PTY.
    TerminalOutput { data: Vec<u8> },
    /// State change notification (e.g. Booting -> Running).
    StateChanged { state: String, trigger: String },
}
