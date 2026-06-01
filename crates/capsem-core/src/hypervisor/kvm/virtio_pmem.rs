//! Virtio pmem device (type 27) for EROFS DAX rootfs experiments.
//!
//! The data plane is a guest-physical memory range registered with KVM. The
//! virtio device only advertises that range and completes flush requests.

use super::memory::GuestMemoryRef;
use super::virtio_mmio::{QueueConfig, VirtioDevice};
use super::virtio_queue::{VirtQueue, VIRTIO_RING_F_EVENT_IDX};

const VIRTIO_ID_PMEM: u32 = 27;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const QUEUE_SIZE: u16 = 16;
const VIRTIO_PMEM_REQ_TYPE_FLUSH: u32 = 0;

pub(in crate::hypervisor::kvm) struct VirtioPmemDevice {
    start: u64,
    size: u64,
    mem: Option<GuestMemoryRef>,
    queue: Option<VirtQueue>,
}

impl VirtioPmemDevice {
    pub fn new(start: u64, size: u64) -> Self {
        Self {
            start,
            size,
            mem: None,
            queue: None,
        }
    }

    fn process_queue(&mut self) -> bool {
        let Some(mem) = self.mem.as_ref() else {
            return false;
        };
        let Some(queue) = self.queue.as_mut() else {
            return false;
        };
        let mut processed = 0u32;
        while let Some(chain) = queue.pop_or_enable_notification() {
            let descs = &chain.descriptors;
            let status =
                if descs.len() == 2 && !descs[0].is_write_only() && descs[1].is_write_only() {
                    0_u32
                } else {
                    libc::EIO as u32
                };

            if let Some(resp_desc) = descs.last().filter(|d| d.is_write_only() && d.len >= 4) {
                if let Some(ptr) = mem.gpa_range_to_host(resp_desc.addr, 4) {
                    unsafe {
                        std::ptr::write_unaligned(ptr as *mut u32, status.to_le());
                    }
                }
            }

            queue.push_used_deferred(chain.head, 4);
            processed += 1;
        }
        if processed > 0 {
            queue.flush_used();
        }
        processed > 0 && queue.prepare_kick()
    }
}

impl VirtioDevice for VirtioPmemDevice {
    fn device_type(&self) -> u32 {
        VIRTIO_ID_PMEM
    }

    fn features(&self) -> u64 {
        VIRTIO_F_VERSION_1 | VIRTIO_RING_F_EVENT_IDX
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &[QUEUE_SIZE]
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        let mut config = [0u8; 16];
        config[0..8].copy_from_slice(&self.start.to_le_bytes());
        config[8..16].copy_from_slice(&self.size.to_le_bytes());
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = config.get(offset as usize + i).copied().unwrap_or_default();
        }
    }

    fn write_config(&self, _offset: u64, _data: &[u8]) {}

    fn activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) {
        self.mem = Some(mem.clone());
        self.queue = queues.first().filter(|q| q.size > 0).map(|q| {
            if q.warm_restore {
                VirtQueue::new_restored_with_event_idx(
                    mem,
                    q.desc_addr,
                    q.driver_addr,
                    q.device_addr,
                    q.size,
                    q.event_idx,
                )
            } else {
                VirtQueue::new_with_event_idx(
                    mem,
                    q.desc_addr,
                    q.driver_addr,
                    q.device_addr,
                    q.size,
                    q.event_idx,
                )
            }
        });
    }

    fn queue_notify(&mut self, queue_index: u32) -> bool {
        if queue_index != 0 {
            return false;
        }
        self.process_queue()
    }

    fn uses_mmio_interrupt(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmem_device_reports_config_space() {
        let dev = VirtioPmemDevice::new(0x1_0000_0000, 0x2000);
        assert_eq!(dev.device_type(), 27);
        assert_eq!(dev.queue_max_sizes(), &[16]);

        let mut data = [0u8; 16];
        dev.read_config(0, &mut data);
        assert_eq!(
            u64::from_le_bytes(data[0..8].try_into().unwrap()),
            0x1_0000_0000
        );
        assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), 0x2000);
    }

    #[test]
    fn pmem_device_advertises_modern_event_idx() {
        let dev = VirtioPmemDevice::new(0, 4096);
        assert_ne!(dev.features() & VIRTIO_F_VERSION_1, 0);
        assert_ne!(dev.features() & VIRTIO_RING_F_EVENT_IDX, 0);
    }

    #[test]
    fn pmem_flush_request_type_matches_linux_uapi() {
        assert_eq!(VIRTIO_PMEM_REQ_TYPE_FLUSH, 0);
    }
}
