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
use tracing::{debug, warn};

/// A serial console reader that pipes VM output into a broadcast channel.
pub struct SerialConsole {
    tx: broadcast::Sender<Vec<u8>>,
    read_fd: RawFd,
    // Keep the NSPipes alive so the Virtualization framework's file handles stay valid.
    #[allow(dead_code)]
    _pipes: Option<(Retained<NSPipe>, Retained<NSPipe>)>,
}

/// Create a serial port configuration backed by NSPipe pairs.
///
/// Returns the ObjC serial port config, a SerialConsole for reading output,
/// and a RawFd for writing input to the guest (host -> guest).
pub fn create_serial_port() -> Result<(
    Retained<VZVirtioConsoleDeviceSerialPortConfiguration>,
    SerialConsole,
    RawFd,
)> {
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
    let console = SerialConsole {
        tx,
        read_fd: output_read_fd_dup,
        _pipes: Some((input_pipe, output_pipe)),
    };

    Ok((serial_config, console, input_write_fd_dup))
}

/// Create a SerialConsole from raw pipe file descriptors (for testing).
pub fn create_console_from_fd(read_fd: RawFd) -> SerialConsole {
    let (tx, _rx) = broadcast::channel(256);
    SerialConsole { tx, read_fd, _pipes: None }
}

impl SerialConsole {
    /// Subscribe to serial output bytes.
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.tx.subscribe()
    }

    /// Spawn a background thread that reads from the pipe and broadcasts raw bytes.
    pub fn spawn_reader(self) {
        std::thread::spawn(move || {
            // Keep pipes alive for the duration of the reader thread
            let _keep_alive = self._pipes;
            read_loop(self.read_fd, &self.tx);
        });
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

    #[test]
    fn reader_broadcasts_complete_lines() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd);
        let mut rx = console.subscribe();

        console.spawn_reader();

        // Write data to the pipe
        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        writer.write_all(b"hello world\n").unwrap();
        writer.write_all(b"second line\n").unwrap();
        drop(writer); // close triggers EOF

        // Collect lines
        let mut lines = Vec::new();
        loop {
            match rx.blocking_recv() {
                Ok(line) => lines.push(line),
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }

        assert_eq!(lines, vec!["hello world", "second line"]);
    }

    #[test]
    fn reader_handles_partial_lines() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd);
        let mut rx = console.subscribe();

        console.spawn_reader();

        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        // Write partial line, then complete it
        writer.write_all(b"partial").unwrap();
        writer.write_all(b" complete\n").unwrap();
        drop(writer);

        let mut lines = Vec::new();
        loop {
            match rx.blocking_recv() {
                Ok(line) => lines.push(line),
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }

        assert_eq!(lines, vec!["partial complete"]);
    }

    #[test]
    fn reader_flushes_trailing_partial_on_eof() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd);
        let mut rx = console.subscribe();

        console.spawn_reader();

        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        writer.write_all(b"first\nno newline at end").unwrap();
        drop(writer);

        let mut lines = Vec::new();
        loop {
            match rx.blocking_recv() {
                Ok(line) => lines.push(line),
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }

        assert_eq!(lines, vec!["first", "no newline at end"]);
    }

    #[test]
    fn reader_handles_empty_lines() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd);
        let mut rx = console.subscribe();

        console.spawn_reader();

        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        writer.write_all(b"a\n\nb\n").unwrap();
        drop(writer);

        let mut lines = Vec::new();
        loop {
            match rx.blocking_recv() {
                Ok(line) => lines.push(line),
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }

        assert_eq!(lines, vec!["a", "", "b"]);
    }

    #[test]
    fn reader_handles_immediate_eof() {
        let (read_fd, write_fd) = make_pipe();
        let console = create_console_from_fd(read_fd);
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
        let console = create_console_from_fd(read_fd);

        let _rx1 = console.subscribe();
        let _rx2 = console.subscribe();
        // Multiple subscribers should work without panic
    }

    #[test]
    fn create_serial_port_returns_valid_config() {
        let (config, _console, input_fd) = create_serial_port().unwrap();
        // The config should have an attachment set
        let attachment = unsafe { config.attachment() };
        assert!(attachment.is_some());
        // input_fd should be a valid file descriptor
        assert!(input_fd >= 0);
        unsafe { libc::close(input_fd); }
    }
}
