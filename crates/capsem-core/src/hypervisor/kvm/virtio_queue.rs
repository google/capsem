//! Split virtqueue implementation.
//!
//! Operates on guest memory directly: descriptor table, available ring, used ring.
//! No external virtio-queue crate -- this is ~300 lines of focused code.

use std::sync::atomic::{fence, Ordering};

use tracing::debug;

use super::memory::GuestMemoryRef;

// ---------------------------------------------------------------------------
// Virtio descriptor flags
// ---------------------------------------------------------------------------

/// Descriptor continues in the `next` field.
pub(super) const VRING_DESC_F_NEXT: u16 = 1;
/// Descriptor buffer is device-writable (host writes, guest reads).
pub(super) const VRING_DESC_F_WRITE: u16 = 2;
/// Driver requests that the device avoid used-buffer interrupts.
const VRING_AVAIL_F_NO_INTERRUPT: u16 = 1;
/// Virtio ring event-index feature bit.
pub(super) const VIRTIO_RING_F_EVENT_IDX: u64 = 1 << 29;

// ---------------------------------------------------------------------------
// Virtqueue descriptor (16 bytes in guest memory)
// ---------------------------------------------------------------------------

/// A single virtqueue descriptor.
#[derive(Debug, Clone, Copy)]
pub(super) struct VirtqDesc {
    pub addr: u64,  // guest physical address of buffer
    pub len: u32,   // buffer length
    pub flags: u16, // VRING_DESC_F_*
    pub next: u16,  // next descriptor index (if NEXT flag set)
}

impl VirtqDesc {
    fn read_from(mem: &GuestMemoryRef, desc_table_gpa: u64, index: u16) -> Option<Self> {
        let offset = desc_table_gpa + u64::from(index) * 16;
        let host = mem.gpa_to_host(offset)?;
        unsafe {
            let addr = u64::from_le(std::ptr::read_unaligned(host as *const u64));
            let len = u32::from_le(std::ptr::read_unaligned(host.cast_const().add(8) as *const u32));
            let flags = u16::from_le(std::ptr::read_unaligned(host.cast_const().add(12) as *const u16));
            let next = u16::from_le(std::ptr::read_unaligned(host.cast_const().add(14) as *const u16));
            Some(VirtqDesc { addr, len, flags, next })
        }
    }

    pub fn is_write_only(&self) -> bool {
        self.flags & VRING_DESC_F_WRITE != 0
    }

    pub fn has_next(&self) -> bool {
        self.flags & VRING_DESC_F_NEXT != 0
    }
}

// ---------------------------------------------------------------------------
// Descriptor chain
// ---------------------------------------------------------------------------

/// A chain of descriptors starting from a head index.
#[derive(Debug)]
pub(super) struct DescriptorChain {
    pub head: u16,
    pub descriptors: Vec<VirtqDesc>,
}

// ---------------------------------------------------------------------------
// VirtQueue
// ---------------------------------------------------------------------------

/// A split virtqueue with descriptor table, available ring, and used ring.
pub(super) struct VirtQueue {
    desc_table_gpa: u64,
    avail_ring_gpa: u64,
    used_ring_gpa: u64,
    size: u16,
    next_avail: u16,
    next_used: u16,
    num_added: u16,
    event_idx: bool,
    mem: GuestMemoryRef,
}

#[derive(Debug, Clone, Copy)]
struct QueueIndices {
    desc_table_gpa: u64,
    avail_ring_gpa: u64,
    used_ring_gpa: u64,
    size: u16,
    next_avail: u16,
    next_used: u16,
    event_idx: bool,
}

impl VirtQueue {
    /// Create a new virtqueue from guest-provided addresses.
    pub fn new(mem: GuestMemoryRef, desc_table_gpa: u64, avail_ring_gpa: u64, used_ring_gpa: u64, size: u16) -> Self {
        let next_used = read_u16(&mem, used_ring_gpa + 2);
        Self::from_indices(
            mem,
            QueueIndices {
                desc_table_gpa,
                avail_ring_gpa,
                used_ring_gpa,
                size,
                next_avail: next_used,
                next_used,
                event_idx: false,
            },
        )
    }

    /// Create a new virtqueue and enable event-index notification suppression
    /// when the driver negotiated `VIRTIO_RING_F_EVENT_IDX`.
    pub fn new_with_event_idx(
        mem: GuestMemoryRef,
        desc_table_gpa: u64,
        avail_ring_gpa: u64,
        used_ring_gpa: u64,
        size: u16,
        event_idx: bool,
    ) -> Self {
        let next_used = read_u16(&mem, used_ring_gpa + 2);
        Self::from_indices(
            mem,
            QueueIndices {
                desc_table_gpa,
                avail_ring_gpa,
                used_ring_gpa,
                size,
                next_avail: next_used,
                next_used,
                event_idx,
            },
        )
    }

    /// Recreate a queue after warm restore.
    ///
    /// KVM checkpoints are taken after device quiescence. Descriptor heads that
    /// were visible before suspend have either already been completed by the
    /// pre-suspend device instance or belong to backend-specific standing
    /// buffers. Replaying them through a fresh userspace device can wedge
    /// VirtioFS after resume, so restored queues wait for the next driver
    /// submission while preserving the used-ring index for future completions.
    pub fn new_restored(
        mem: GuestMemoryRef,
        desc_table_gpa: u64,
        avail_ring_gpa: u64,
        used_ring_gpa: u64,
        size: u16,
    ) -> Self {
        let next_avail = read_u16(&mem, avail_ring_gpa + 2);
        let next_used = read_u16(&mem, used_ring_gpa + 2);
        debug!(
            event_name = "virtio.queue.restore",
            desc_table_gpa, avail_ring_gpa, used_ring_gpa, size, next_avail, next_used, "virtqueue restored"
        );
        Self::from_indices(
            mem,
            QueueIndices {
                desc_table_gpa,
                avail_ring_gpa,
                used_ring_gpa,
                size,
                next_avail,
                next_used,
                event_idx: false,
            },
        )
    }

    /// Recreate a queue after warm restore with event-index enabled when it
    /// was negotiated before activation.
    pub fn new_restored_with_event_idx(
        mem: GuestMemoryRef,
        desc_table_gpa: u64,
        avail_ring_gpa: u64,
        used_ring_gpa: u64,
        size: u16,
        event_idx: bool,
    ) -> Self {
        let next_avail = read_u16(&mem, avail_ring_gpa + 2);
        let next_used = read_u16(&mem, used_ring_gpa + 2);
        debug!(
            event_name = "virtio.queue.restore",
            desc_table_gpa, avail_ring_gpa, used_ring_gpa, size, next_avail, next_used, event_idx, "virtqueue restored"
        );
        Self::from_indices(
            mem,
            QueueIndices {
                desc_table_gpa,
                avail_ring_gpa,
                used_ring_gpa,
                size,
                next_avail,
                next_used,
                event_idx,
            },
        )
    }

    fn from_indices(mem: GuestMemoryRef, indices: QueueIndices) -> Self {
        Self {
            desc_table_gpa: indices.desc_table_gpa,
            avail_ring_gpa: indices.avail_ring_gpa,
            used_ring_gpa: indices.used_ring_gpa,
            size: indices.size,
            next_avail: indices.next_avail,
            next_used: indices.next_used,
            num_added: 0,
            event_idx: indices.event_idx,
            mem,
        }
    }

    /// Pop the next available descriptor chain, if any.
    ///
    /// Returns None if no descriptors are available (ring empty).
    pub fn pop(&mut self) -> Option<DescriptorChain> {
        // Acquire: ensure we see descriptor writes made by the driver
        // before the avail index update. Required by virtio spec when
        // device and driver run on different threads.
        fence(Ordering::Acquire);
        let avail_idx = self.read_avail_idx();
        if self.next_avail == avail_idx {
            return None; // ring empty
        }

        // Read the descriptor head index from the avail ring
        let ring_index = self.next_avail % self.size;
        let head = self.read_avail_ring(ring_index);
        self.next_avail = self.next_avail.wrapping_add(1);

        // Walk the descriptor chain
        let mut descriptors = Vec::new();
        let mut idx = head;
        let mut visited = 0u32;

        loop {
            if visited >= u32::from(self.size) {
                // Cycle detection: we've visited more descriptors than the queue size
                break;
            }

            let desc = VirtqDesc::read_from(&self.mem, self.desc_table_gpa, idx)?;
            descriptors.push(desc);
            visited += 1;

            if !desc.has_next() {
                break;
            }
            idx = desc.next;
        }

        Some(DescriptorChain { head, descriptors })
    }

    /// Pop a descriptor chain, or arm driver notifications if the queue is empty.
    ///
    /// With event-index negotiated, this follows the Firecracker/Linux pattern:
    /// when the queue looks empty, write `avail_event = next_avail`, fence, and
    /// recheck `avail.idx`. If the driver raced by publishing a descriptor
    /// before seeing the armed event index, the second read catches it and the
    /// worker keeps draining instead of sleeping forever.
    pub fn pop_or_enable_notification(&mut self) -> Option<DescriptorChain> {
        if !self.event_idx {
            return self.pop();
        }

        if let Some(chain) = self.pop() {
            return Some(chain);
        }

        self.write_avail_event(self.next_avail);
        fence(Ordering::SeqCst);

        self.pop()
    }

    /// Push a used descriptor chain back to the used ring.
    pub fn push_used(&mut self, head: u16, len: u32) {
        self.push_used_deferred(head, len);
        self.flush_used();
    }

    /// Push a used descriptor without publishing the used index yet.
    ///
    /// Devices that complete multiple descriptor chains from one notification
    /// can call this repeatedly and publish them with one `flush_used()`.
    pub fn push_used_deferred(&mut self, head: u16, len: u32) {
        let used_index = self.next_used % self.size;
        self.write_used_ring(used_index, head, len);
        self.next_used = self.next_used.wrapping_add(1);
        self.num_added = self.num_added.wrapping_add(1);
    }

    /// Publish all deferred used ring entries to the driver.
    pub fn flush_used(&mut self) {
        // Release: ensure used ring entry writes are visible to the driver
        // before the used index update. Required by virtio spec when
        // device and driver run on different threads.
        fence(Ordering::Release);
        self.write_used_idx(self.next_used);
    }

    /// Decide whether the driver should be interrupted after used entries were published.
    ///
    /// This is the split-ring `prepare_kick` step. Without event-index, the
    /// legacy `NO_INTERRUPT` flag controls suppression. With event-index, the
    /// driver-owned `used_event` field tells the device which used index should
    /// trigger the next interrupt.
    pub fn prepare_kick(&mut self) -> bool {
        if self.num_added == 0 {
            return false;
        }

        if !self.event_idx {
            self.num_added = 0;
            return self.read_avail_flags() & VRING_AVAIL_F_NO_INTERRUPT == 0;
        }

        fence(Ordering::SeqCst);

        let new = self.next_used;
        let old = self.next_used.wrapping_sub(self.num_added);
        let used_event = self.read_used_event();
        self.num_added = 0;

        new.wrapping_sub(used_event).wrapping_sub(1) < new.wrapping_sub(old)
    }

    /// Read the `idx` field from the available ring.
    fn read_avail_idx(&self) -> u16 {
        // avail ring layout: flags (u16), idx (u16), ring[size] (u16 each)
        let idx_gpa = self.avail_ring_gpa + 2; // skip flags
        if let Some(ptr) = self.mem.gpa_to_host(idx_gpa) {
            unsafe { u16::from_le(std::ptr::read_unaligned(ptr as *const u16)) }
        } else {
            0
        }
    }

    /// Read the `flags` field from the available ring.
    fn read_avail_flags(&self) -> u16 {
        read_u16(&self.mem, self.avail_ring_gpa)
    }

    /// Read a ring entry from the available ring.
    fn read_avail_ring(&self, ring_index: u16) -> u16 {
        // ring entries start at offset 4 (after flags + idx)
        let entry_gpa = self.avail_ring_gpa + 4 + u64::from(ring_index) * 2;
        if let Some(ptr) = self.mem.gpa_to_host(entry_gpa) {
            unsafe { u16::from_le(std::ptr::read_unaligned(ptr as *const u16)) }
        } else {
            0
        }
    }

    /// Read `used_event` from the end of the available ring.
    fn read_used_event(&self) -> u16 {
        read_u16(&self.mem, self.avail_ring_gpa + 4 + u64::from(self.size) * 2)
    }

    /// Write `avail_event` at the end of the used ring.
    fn write_avail_event(&self, idx: u16) {
        let event_gpa = self.used_ring_gpa + 4 + u64::from(self.size) * 8;
        if let Some(ptr) = self.mem.gpa_to_host(event_gpa) {
            unsafe {
                std::ptr::write_unaligned(ptr as *mut u16, idx.to_le());
            }
        }
    }

    /// Write a used ring entry.
    fn write_used_ring(&self, ring_index: u16, id: u16, len: u32) {
        // used ring layout: flags (u16), idx (u16), ring[size] {id: u32, len: u32}
        let entry_gpa = self.used_ring_gpa + 4 + u64::from(ring_index) * 8;
        if let Some(ptr) = self.mem.gpa_to_host(entry_gpa) {
            unsafe {
                std::ptr::write_unaligned(ptr as *mut u32, u32::from(id).to_le());
                std::ptr::write_unaligned(ptr.add(4) as *mut u32, len.to_le());
            }
        }
    }

    /// Write the `idx` field of the used ring.
    fn write_used_idx(&self, idx: u16) {
        let idx_gpa = self.used_ring_gpa + 2; // skip flags
        if let Some(ptr) = self.mem.gpa_to_host(idx_gpa) {
            unsafe {
                std::ptr::write_unaligned(ptr as *mut u16, idx.to_le());
            }
        }
    }
}

fn read_u16(mem: &GuestMemoryRef, gpa: u64) -> u16 {
    mem.gpa_to_host(gpa).map_or(0, |ptr| unsafe {
        u16::from_le(std::ptr::read_unaligned(ptr as *const u16))
    })
}

#[cfg(test)]
mod tests;
