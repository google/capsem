use std::io::Read;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
#[cfg(test)]
use std::os::fd::{FromRawFd, RawFd};

use anyhow::{Context, Result};
use objc2::rc::Retained;
use objc2::AllocAnyThread;
use objc2_foundation::NSPipe;
use objc2_virtualization::{VZFileHandleSerialPortAttachment, VZVirtioConsoleDeviceSerialPortConfiguration};
use tokio::sync::broadcast;
use tracing::{debug, debug_span, warn};

/// A serial console reader that pipes VM output into a broadcast channel.
pub struct AppleVzSerialConsole {
    tx: broadcast::Sender<Vec<u8>>,
    read_fd: OwnedFd,
    input_fd: Option<OwnedFd>,
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
    // SAFETY: NSPipe owns this descriptor for the duration of the duplicate call.
    let output_read_fd = unsafe { BorrowedFd::borrow_raw(output_read_fd) };
    let output_read_fd_dup =
        capsem_foundation::unix::fd::duplicate(output_read_fd).context("duplicate Apple VZ output pipe")?;

    // Get the raw fd for the host-owned input pipe writer.
    let input_write_fd = input_pipe.fileHandleForWriting().fileDescriptor();
    // SAFETY: NSPipe owns this descriptor for the duration of the duplicate call.
    let input_write_fd = unsafe { BorrowedFd::borrow_raw(input_write_fd) };
    let input_write_fd_dup =
        capsem_foundation::unix::fd::duplicate(input_write_fd).context("duplicate Apple VZ input pipe")?;

    let (tx, _rx) = broadcast::channel(256);
    let console = AppleVzSerialConsole {
        tx,
        read_fd: output_read_fd_dup,
        input_fd: Some(input_write_fd_dup),
        _pipes: Some((input_pipe, output_pipe)),
    };

    Ok((serial_config, console))
}

/// Create an AppleVzSerialConsole that owns raw pipe file descriptors (for testing).
#[cfg(test)]
pub fn create_console_from_fd(read_fd: RawFd, input_fd: RawFd) -> AppleVzSerialConsole {
    let (tx, _rx) = broadcast::channel(256);
    AppleVzSerialConsole {
        tx,
        read_fd: unsafe { OwnedFd::from_raw_fd(read_fd) },
        input_fd: (input_fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(input_fd) }),
        _pipes: None,
    }
}

impl AppleVzSerialConsole {
    /// Subscribe to serial output bytes (used by tests; production goes through SerialConsole trait).
    #[cfg(test)]
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.tx.subscribe()
    }

    /// Spawn a background thread that reads from the pipe and broadcasts raw bytes.
    pub fn spawn_reader(&self) {
        let read_fd = match self.read_fd.try_clone() {
            Ok(read_fd) => read_fd,
            Err(error) => {
                warn!(%error, "failed to duplicate serial reader fd");
                return;
            }
        };
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
        self.input_fd.as_ref().map_or(-1, |input_fd| input_fd.as_raw_fd())
    }
}

/// Core read loop: reads bytes from a file descriptor and sends them
/// immediately through the broadcast channel.
fn read_loop(fd: OwnedFd, tx: &broadcast::Sender<Vec<u8>>) {
    let mut file = std::fs::File::from(fd);
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
                warn!(error = %e, "serial read error");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests;
