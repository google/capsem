//! KVM serial console -- pipe-backed broadcast channel.
//!
//! Structurally identical to apple_vz/serial.rs: a pipe pair connects the
//! virtio-console device to the SerialConsole trait. A background thread
//! reads from the guest-output pipe and broadcasts via tokio broadcast.

use std::io::Read;
use std::os::unix::io::{FromRawFd, RawFd};

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
        Self { tx, read_fd, input_fd }
    }

    /// Subscribe to serial output bytes.
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.tx.subscribe()
    }

    /// Spawn a background thread that reads from the pipe and broadcasts.
    pub fn spawn_reader(&self) {
        let read_fd = self.read_fd;
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("kvm-serial-reader".to_string())
            .spawn(move || {
                read_loop(read_fd, &tx);
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
fn read_loop(fd: RawFd, tx: &broadcast::Sender<Vec<u8>>) {
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut buf = [0u8; 4096];

    loop {
        match file.read(&mut buf) {
            Ok(0) => {
                debug!("KVM serial console EOF");
                break;
            }
            Ok(n) => {
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
mod tests {
    use super::*;
    use std::io::Write;

    fn make_pipe() -> (RawFd, RawFd) {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    }

    fn collect_all(rx: &mut broadcast::Receiver<Vec<u8>>) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            match rx.blocking_recv() {
                Ok(chunk) => out.extend_from_slice(&chunk),
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
        out
    }

    #[test]
    fn reader_broadcasts_data() {
        let (read_fd, write_fd) = make_pipe();
        let console = KvmSerialConsole::new(read_fd, -1);
        let mut rx = console.subscribe();
        console.spawn_reader();
        drop(console); // drop sender so collect_all gets Closed

        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        writer.write_all(b"hello world\n").unwrap();
        writer.write_all(b"second line\n").unwrap();
        drop(writer);

        let all = collect_all(&mut rx);
        assert_eq!(all, b"hello world\nsecond line\n");
    }

    #[test]
    fn reader_handles_partial_writes() {
        let (read_fd, write_fd) = make_pipe();
        let console = KvmSerialConsole::new(read_fd, -1);
        let mut rx = console.subscribe();
        console.spawn_reader();
        drop(console);

        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        writer.write_all(b"partial").unwrap();
        writer.write_all(b" complete\n").unwrap();
        drop(writer);

        let all = collect_all(&mut rx);
        assert_eq!(all, b"partial complete\n");
    }

    #[test]
    fn reader_handles_immediate_eof() {
        let (read_fd, write_fd) = make_pipe();
        let console = KvmSerialConsole::new(read_fd, -1);
        let mut rx = console.subscribe();

        unsafe { libc::close(write_fd); }
        console.spawn_reader();

        std::thread::sleep(std::time::Duration::from_millis(50));
        match rx.try_recv() {
            Err(broadcast::error::TryRecvError::Closed) => {}
            Err(broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected closed or empty, got {other:?}"),
        }
    }

    #[test]
    fn multiple_subscribers() {
        let (read_fd, write_fd) = make_pipe();
        let console = KvmSerialConsole::new(read_fd, -1);
        let _rx1 = console.subscribe();
        let _rx2 = console.subscribe();
        // Should not panic
        unsafe { libc::close(write_fd); }
    }

    #[test]
    fn input_fd_returns_stored_value() {
        let (read_fd, write_fd) = make_pipe();
        let console = KvmSerialConsole::new(read_fd, write_fd);
        let trait_ref: &dyn crate::hypervisor::SerialConsole = &console;
        assert_eq!(trait_ref.input_fd(), write_fd);
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
    }

    #[test]
    fn serial_console_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<KvmSerialConsole>();
    }

    #[test]
    fn negative_input_fd() {
        let (read_fd, _write_fd) = make_pipe();
        let console = KvmSerialConsole::new(read_fd, -1);
        let trait_ref: &dyn crate::hypervisor::SerialConsole = &console;
        assert_eq!(trait_ref.input_fd(), -1);
    }
}
