//! Virtio console device (type 3) for hvc0.
//!
//! Two queues: receiveq (host->guest) and transmitq (guest->host).
//! Backed by a pipe pair for integration with KvmSerialConsole.

use std::io::Write;
use std::os::unix::io::{FromRawFd, RawFd};

use anyhow::{Result, bail};

use super::memory::GuestMemoryRef;
use super::serial::KvmSerialConsole;
use super::virtio_mmio::{QueueConfig, VirtioDevice};

/// Virtio console device ID.
const VIRTIO_ID_CONSOLE: u32 = 3;

/// Maximum queue size.
const QUEUE_SIZE: u16 = 256;

/// Virtio console device backed by pipe I/O.
pub(super) struct VirtioConsoleDevice {
    /// Write end of the output pipe (guest output -> host reads).
    tx_fd: RawFd,
}

impl VirtioConsoleDevice {
    /// Create a new virtio console device and its associated serial console.
    ///
    /// Returns (device, serial_console):
    /// - device: the VirtioDevice to register with MMIO transport
    /// - serial_console: the SerialConsole trait impl for the hypervisor handle
    pub fn new() -> Result<(Self, KvmSerialConsole)> {
        // Output pipe: guest writes -> host reads (broadcast)
        let (output_read_fd, output_write_fd) = make_pipe()?;

        // Input pipe: host writes -> guest reads
        let (input_read_fd, input_write_fd) = make_pipe()?;

        let device = Self {
            tx_fd: output_write_fd,
        };

        let console = KvmSerialConsole::new(output_read_fd, input_write_fd);

        // Close fds we don't need in this context
        // input_read_fd will be used when we implement receiveq processing
        // For now, we leak it (it stays open for the kernel to read from)
        let _ = input_read_fd;

        Ok((device, console))
    }
}

impl VirtioDevice for VirtioConsoleDevice {
    fn device_type(&self) -> u32 {
        VIRTIO_ID_CONSOLE
    }

    fn features(&self) -> u64 {
        // VIRTIO_F_VERSION_1 (bit 32) -- modern virtio
        1u64 << 32
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &[QUEUE_SIZE, QUEUE_SIZE] // receiveq, transmitq
    }

    fn read_config(&self, _offset: u64, data: &mut [u8]) {
        // Minimal config: no multiport, no emerg_wr
        data.fill(0);
    }

    fn write_config(&self, _offset: u64, _data: &[u8]) {
        // No writable config
    }

    fn activate(&mut self, _mem: GuestMemoryRef, _queues: &[QueueConfig]) {
        // Device is now active -- queue processing will happen on notify
    }

    fn queue_notify(&mut self, queue_index: u32) {
        if queue_index == 1 {
            // transmitq: guest has data for us
            // In a full implementation, we'd pop from the transmitq and write to tx_fd.
            // For now, this is a placeholder -- actual queue processing will be added
            // when the vCPU run loop is fully integrated.
            // TODO: pop descriptor chains from transmitq, write data to tx_fd
        }
    }
}

impl Drop for VirtioConsoleDevice {
    fn drop(&mut self) {
        if self.tx_fd >= 0 {
            unsafe { libc::close(self.tx_fd); }
        }
    }
}

fn make_pipe() -> Result<(RawFd, RawFd)> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        bail!("pipe() failed: {}", std::io::Error::last_os_error());
    }
    Ok((fds[0], fds[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::io::FromRawFd;

    #[test]
    fn console_device_type() {
        let (dev, _console) = VirtioConsoleDevice::new().unwrap();
        assert_eq!(dev.device_type(), VIRTIO_ID_CONSOLE);
    }

    #[test]
    fn console_features() {
        let (dev, _console) = VirtioConsoleDevice::new().unwrap();
        let features = dev.features();
        // VIRTIO_F_VERSION_1 should be set
        assert_ne!(features & (1 << 32), 0);
    }

    #[test]
    fn console_has_two_queues() {
        let (dev, _console) = VirtioConsoleDevice::new().unwrap();
        assert_eq!(dev.queue_max_sizes().len(), 2);
        assert_eq!(dev.queue_max_sizes()[0], 256);
        assert_eq!(dev.queue_max_sizes()[1], 256);
    }

    #[test]
    fn console_config_is_zero() {
        let (dev, _console) = VirtioConsoleDevice::new().unwrap();
        let mut data = [0xFFu8; 16];
        dev.read_config(0, &mut data);
        assert!(data.iter().all(|&b| b == 0));
    }

    #[test]
    fn console_creates_working_pipe() {
        // Verify the pipe pair works: write to tx_fd, read from console's output
        let (dev, console) = VirtioConsoleDevice::new().unwrap();

        // Write to the device's tx pipe
        let mut writer = unsafe { std::fs::File::from_raw_fd(dev.tx_fd) };
        writer.write_all(b"hello from guest").unwrap();
        // Don't close writer yet -- drop will close tx_fd
        std::mem::forget(writer); // let Drop on VirtioConsoleDevice handle it

        // Subscribe and verify data arrives via the console
        let mut rx = console.subscribe();
        console.spawn_reader();

        // Give the reader thread a moment
        std::thread::sleep(std::time::Duration::from_millis(50));

        // We need to close the write end to trigger EOF in the reader
        // The device will close tx_fd on drop
        drop(dev);

        // Collect what was broadcast
        let mut all = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(chunk) => all.extend_from_slice(&chunk),
                Err(_) => break,
            }
        }
        assert_eq!(all, b"hello from guest");
    }

    #[test]
    fn console_serial_input_fd_valid() {
        let (_dev, console) = VirtioConsoleDevice::new().unwrap();
        let fd = crate::hypervisor::SerialConsole::input_fd(&console);
        assert!(fd >= 0, "input_fd should be non-negative");
    }
}
