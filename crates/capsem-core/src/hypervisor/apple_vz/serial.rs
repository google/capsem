use std::io::Read;
use std::os::unix::io::{FromRawFd, RawFd};

use anyhow::Result;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_foundation::NSPipe;
use objc2_virtualization::{
    VZFileHandleSerialPortAttachment, VZVirtioConsoleDeviceSerialPortConfiguration,
};
use tokio::sync::broadcast;
use tracing::{debug, debug_span, warn};

/// A serial console reader that pipes VM output into a broadcast channel.
pub struct AppleVzSerialConsole {
    tx: broadcast::Sender<Vec<u8>>,
    read_fd: RawFd,
    input_fd: RawFd,
    // Keep the NSPipes alive so the Virtualization framework's file handles stay valid.
    #[allow(dead_code)]
    _pipes: Option<(Retained<NSPipe>, Retained<NSPipe>)>,
}

/// Create a serial port configuration backed by NSPipe pairs.
///
/// Returns the ObjC serial port config and an AppleVzSerialConsole
/// that owns both the read (output) and write (input) file descriptors.
pub fn create_serial_port() -> Result<(
    Retained<VZVirtioConsoleDeviceSerialPortConfiguration>,
    AppleVzSerialConsole,
)> {
    let _span = debug_span!("create_serial_port").entered();
    // Input pipe: host writes to inputPipe.fileHandleForWriting,
    //             framework reads from inputPipe.fileHandleForReading -> guest
    let input_pipe = NSPipe::pipe();

    // Output pipe: guest -> framework writes to outputPipe.fileHandleForWriting,
    //              host reads from outputPipe.fileHandleForReading
    let output_pipe = NSPipe::pipe();

    let serial_config = unsafe {
        let attachment = VZFileHandleSerialPortAttachment::initWithFileHandleForReading_fileHandleForWriting(
            VZFileHandleSerialPortAttachment::alloc(),
            Some(&input_pipe.fileHandleForReading()),
            Some(&output_pipe.fileHandleForWriting()),
        );

        let config = VZVirtioConsoleDeviceSerialPortConfiguration::new();
        config.setAttachment(Some(&attachment));
        config
    };

    // Get the raw fd for the host-side read end of the output pipe.
    let output_read_fd = output_pipe.fileHandleForReading().fileDescriptor();
    // Dup it so we have our own fd that survives even if NSPipe manages the original.
    let output_read_fd_dup = unsafe { libc::dup(output_read_fd) };
    if output_read_fd_dup < 0 {
        return Err(anyhow::anyhow!(
            "dup() failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    // Get the raw fd for the host-side write end of the input pipe.
    let input_write_fd = input_pipe.fileHandleForWriting().fileDescriptor();
    let input_write_fd_dup = unsafe { libc::dup(input_write_fd) };
    if input_write_fd_dup < 0 {
        unsafe { libc::close(output_read_fd_dup); }
        return Err(anyhow::anyhow!(
            "dup() failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    let (tx, _rx) = broadcast::channel(256);
    let console = AppleVzSerialConsole {
        tx,
        read_fd: output_read_fd_dup,
        input_fd: input_write_fd_dup,
        _pipes: Some((input_pipe, output_pipe)),
    };

    Ok((serial_config, console))
}

/// Create an AppleVzSerialConsole from raw pipe file descriptors (for testing).
pub fn create_console_from_fd(read_fd: RawFd, input_fd: RawFd) -> AppleVzSerialConsole {
    let (tx, _rx) = broadcast::channel(256);
    AppleVzSerialConsole { tx, read_fd, input_fd, _pipes: None }
}

impl AppleVzSerialConsole {
    /// Subscribe to serial output bytes.
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.tx.subscribe()
    }

    /// Spawn a background thread that reads from the pipe and broadcasts raw bytes.
    pub fn spawn_reader(&self) {
        let read_fd = self.read_fd;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            read_loop(read_fd, &tx);
        });
    }
}

impl crate::hypervisor::SerialConsole for AppleVzSerialConsole {
    fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.tx.subscribe()
    }

    fn input_fd(&self) -> RawFd {
        self.input_fd
    }
}

/// Core read loop: reads bytes from a file descriptor and sends them
/// immediately through the broadcast channel.
fn read_loop(fd: RawFd, tx: &broadcast::Sender<Vec<u8>>) {
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut buf = [0u8; 4096];

    loop {
        match file.read(&mut buf) {
            Ok(0) => {
                debug!("serial console EOF");
                break;
            }
            Ok(n) => {
                let _ = tx.send(buf[..n].to_vec());
            }
            Err(e) => {
                warn!("serial read error: {e}");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::io::FromRawFd;
    use std::time::Duration;

    fn make_pipe() -> (RawFd, RawFd) {
        let mut fds = [0 as RawFd; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    }

    /// Collect all broadcast chunks into a single byte vector.
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
    fn reader_broadcasts_written_data() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd, -1);
        let mut rx = console.subscribe();
        console.spawn_reader();
        drop(console); // drop tx so collect_all gets Closed after EOF

        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        writer.write_all(b"hello world\n").unwrap();
        writer.write_all(b"second line\n").unwrap();
        drop(writer);

        let all = collect_all(&mut rx);
        assert_eq!(all, b"hello world\nsecond line\n");
    }

    #[test]
    fn reader_broadcasts_partial_writes() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd, -1);
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
    fn reader_broadcasts_data_without_trailing_newline() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd, -1);
        let mut rx = console.subscribe();
        console.spawn_reader();
        drop(console);

        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        writer.write_all(b"first\nno newline at end").unwrap();
        drop(writer);

        let all = collect_all(&mut rx);
        assert_eq!(all, b"first\nno newline at end");
    }

    #[test]
    fn reader_broadcasts_empty_lines() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd, -1);
        let mut rx = console.subscribe();
        console.spawn_reader();
        drop(console);

        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        writer.write_all(b"a\n\nb\n").unwrap();
        drop(writer);

        let all = collect_all(&mut rx);
        assert_eq!(all, b"a\n\nb\n");
    }

    #[test]
    fn reader_handles_immediate_eof() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd, -1);
        let mut rx = console.subscribe();

        // Close write end immediately
        unsafe { libc::close(write_fd); }

        console.spawn_reader();

        // Should get Closed with no lines
        std::thread::sleep(Duration::from_millis(50));
        match rx.try_recv() {
            Err(broadcast::error::TryRecvError::Closed) => {}
            Err(broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected closed or empty, got {other:?}"),
        }
    }

    #[test]
    fn subscribe_returns_receiver() {
        let (read_fd, _write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd, -1);

        let _rx1 = console.subscribe();
        let _rx2 = console.subscribe();
        // Multiple subscribers should work without panic
    }

    #[test]
    fn create_serial_port_returns_valid_config() {
        let (config, console, ) = create_serial_port().unwrap();
        // The config should have an attachment set
        let attachment = unsafe { config.attachment() };
        assert!(attachment.is_some());
        // input_fd should be a valid file descriptor
        assert!(console.input_fd >= 0);
        unsafe { libc::close(console.input_fd); }
    }

    #[test]
    fn serial_console_trait_input_fd() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd, write_fd);
        let trait_ref: &dyn crate::hypervisor::SerialConsole = &console;
        assert_eq!(trait_ref.input_fd(), write_fd);
    }
}
