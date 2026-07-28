//! KVM serial console -- pipe-backed broadcast channel.
//!
//! Structurally identical to apple_vz/serial.rs: a pipe pair connects the
//! virtio-console device to the SerialConsole trait. A background thread
//! reads from the guest-output pipe and broadcasts via tokio broadcast.

use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, RawFd};
use std::path::PathBuf;

use tokio::sync::broadcast;
use tracing::{debug, warn};

/// Serial console for the KVM backend.
///
/// Wraps a pipe pair: guest output flows through `read_fd` -> broadcast,
/// and host input is written to `input_fd` -> guest.
pub(super) struct KvmSerialConsole {
    tx: broadcast::Sender<Vec<u8>>,
    read_fd: RawFd,
    input_fd: RawFd,
}

// Safety: fds are plain integers usable from any thread.
// The broadcast::Sender is Send+Sync.
unsafe impl Sync for KvmSerialConsole {}

impl KvmSerialConsole {
    /// Create a new serial console from raw pipe fds.
    ///
    /// - `read_fd`: read end of the output pipe (guest output -> host)
    /// - `input_fd`: write end of the input pipe (host -> guest input)
    pub fn new(read_fd: RawFd, input_fd: RawFd) -> Self {
        let (tx, _rx) = broadcast::channel(256);
        Self {
            tx,
            read_fd,
            input_fd,
        }
    }

    /// Subscribe to serial output bytes.
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.tx.subscribe()
    }

    /// Spawn a background thread that reads from the pipe and broadcasts.
    pub fn spawn_reader(&self) {
        self.spawn_reader_with_log(None);
    }

    /// Spawn a background thread that reads from the pipe, optionally mirrors
    /// bytes to a durable serial log, and broadcasts chunks to subscribers.
    pub fn spawn_reader_with_log(&self, log_path: Option<PathBuf>) {
        let read_fd = self.read_fd;
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("kvm-serial-reader".to_string())
            .spawn(move || {
                read_loop(read_fd, &tx, log_path);
            })
            .expect("failed to spawn serial reader thread");
    }
}

impl crate::hypervisor::SerialConsole for KvmSerialConsole {
    fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.tx.subscribe()
    }

    fn input_fd(&self) -> RawFd {
        self.input_fd
    }
}

/// Core read loop: reads bytes from fd and sends through broadcast.
fn read_loop(fd: RawFd, tx: &broadcast::Sender<Vec<u8>>, log_path: Option<PathBuf>) {
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut log_file = log_path.and_then(|path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| {
                warn!(error = %e, path = %path.display(), "failed to open KVM serial log file");
                e
            })
            .ok()
    });
    let mut buf = [0u8; 4096];

    loop {
        match file.read(&mut buf) {
            Ok(0) => {
                debug!("KVM serial console EOF");
                break;
            }
            Ok(n) => {
                if let Some(log_file) = log_file.as_mut() {
                    let _ = log_file.write_all(&buf[..n]);
                }
                let _ = tx.send(buf[..n].to_vec());
            }
            Err(e) => {
                warn!("KVM serial read error: {e}");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests;
