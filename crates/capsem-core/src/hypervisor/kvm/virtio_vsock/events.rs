//! Guest-visible reset events are userspace-owned: Linux vhost owns only RX/TX.
//! A restored host has no old sockets; notify the guest immediately rather than
//! waiting for its heartbeat deadlines to notice the lost connections.

use anyhow::{ensure, Result};

use super::super::memory::GuestMemoryRef;
use super::super::virtio_mmio::QueueConfig;
use super::super::virtio_queue::VirtQueue;

pub(super) const QUEUE_INDEX: usize = 2;
// include/uapi/linux/virtio_vsock.h: VIRTIO_VSOCK_EVENT_TRANSPORT_RESET.
const TRANSPORT_RESET: u32 = 0;

pub(super) struct TransportEvents {
    queue: VirtQueue,
    mem: GuestMemoryRef,
    pub(super) reset_pending: bool,
}

impl TransportEvents {
    pub(super) fn new(mem: GuestMemoryRef, config: &QueueConfig) -> Result<Self> {
        ensure!(
            config.size > 0 && config.size <= 256 && config.size.is_power_of_two(),
            "invalid virtio-vsock event queue size"
        );
        // Event buffers are standing driver submissions, not completed FUSE
        // requests. Start at used.idx so a pre-suspend available buffer survives.
        let queue = VirtQueue::new_with_event_idx(
            mem.clone(),
            config.desc_addr,
            config.driver_addr,
            config.device_addr,
            config.size,
            config.event_idx,
        );
        Ok(Self {
            queue,
            mem,
            reset_pending: false,
        })
    }

    pub(super) fn drain_reset(&mut self) -> Result<bool> {
        if !self.reset_pending {
            return Ok(false);
        }
        let Some(chain) = self.queue.pop_or_enable_notification() else {
            return Ok(false);
        };
        let event = TRANSPORT_RESET.to_le_bytes();
        let mut written = 0usize;
        for descriptor in &chain.descriptors {
            if descriptor.is_write_only() && written < event.len() {
                let count = (event.len() - written).min(descriptor.len as usize);
                let copied = self
                    .mem
                    .write_guest_buffer(descriptor.addr, &event[written..written + count]);
                ensure!(copied == count, "virtio-vsock event buffer escapes guest RAM");
                written += count;
            }
        }
        ensure!(written == event.len(), "virtio-vsock reset event buffer is too short");
        self.queue.push_used(chain.head, written as u32);
        self.reset_pending = false;
        Ok(self.queue.prepare_kick())
    }
}

#[cfg(test)]
mod tests;
