//! Virtio console device (type 3) for hvc0.
//!
//! Two queues: receiveq (host->guest) and transmitq (guest->host).
//! Backed by a pipe pair for integration with KvmSerialConsole.

use std::os::unix::io::{FromRawFd, RawFd};

use anyhow::{bail, Result};

use super::memory::GuestMemoryRef;
use super::serial::KvmSerialConsole;
use super::virtio_mmio::{QueueConfig, VirtioDevice};
use super::virtio_queue::VirtQueue;

/// Virtio console device ID.
const VIRTIO_ID_CONSOLE: u32 = 3;

/// Maximum queue size.
const QUEUE_SIZE: u16 = 256;

/// Virtio console device backed by pipe I/O.
pub(super) struct VirtioConsoleDevice {
    /// Write end of the output pipe (guest output -> host reads).
    tx_fd: RawFd,
    transmitq: Option<VirtQueue>,
    mem: Option<GuestMemoryRef>,
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
            transmitq: None,
            mem: None,
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

    fn activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) {
        if let Some(q) = queues.get(1).filter(|q| q.size > 0) {
            tracing::debug!(
                event_name = "virtio.console.activate",
                transmitq_size = q.size,
                transmitq_desc_addr = q.desc_addr,
                transmitq_driver_addr = q.driver_addr,
                transmitq_device_addr = q.device_addr,
                "virtio-console transmit queue activated"
            );
            self.transmitq = Some(if q.warm_restore {
                VirtQueue::new_restored(
                    mem.clone(),
                    q.desc_addr,
                    q.driver_addr,
                    q.device_addr,
                    q.size,
                )
            } else {
                VirtQueue::new(
                    mem.clone(),
                    q.desc_addr,
                    q.driver_addr,
                    q.device_addr,
                    q.size,
                )
            });
        }
        self.mem = Some(mem);
    }

    fn queue_notify(&mut self, queue_index: u32) -> bool {
        let mut completed = false;
        if queue_index == 1 {
            let Some(mem) = self.mem.as_ref() else {
                return false;
            };
            let Some(queue) = self.transmitq.as_mut() else {
                return false;
            };
            while let Some(chain) = queue.pop() {
                let mut written = 0u32;
                for desc in &chain.descriptors {
                    if desc.is_write_only() {
                        continue;
                    }
                    if let Some(ptr) = mem.gpa_to_host(desc.addr) {
                        let mut offset = 0usize;
                        while offset < desc.len as usize {
                            let ret = unsafe {
                                libc::write(
                                    self.tx_fd,
                                    ptr.add(offset) as *const libc::c_void,
                                    desc.len as usize - offset,
                                )
                            };
                            if ret <= 0 {
                                tracing::warn!(
                                    event_name = "virtio.console.write_error",
                                    errno = %std::io::Error::last_os_error(),
                                    "failed to write guest console output"
                                );
                                break;
                            }
                            offset += ret as usize;
                        }
                        written = written.saturating_add(offset as u32);
                    }
                }
                tracing::trace!(
                    event_name = "virtio.console.transmit_complete",
                    head = chain.head,
                    bytes = written,
                    "virtio-console transmit descriptor completed"
                );
                queue.push_used(chain.head, written);
                completed = true;
            }
        }
        completed
    }

    fn uses_mmio_interrupt(&self) -> bool {
        true
    }
}

impl Drop for VirtioConsoleDevice {
    fn drop(&mut self) {
        if self.tx_fd >= 0 {
            unsafe {
                libc::close(self.tx_fd);
            }
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
mod tests;
