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
        let offset = desc_table_gpa + (index as u64) * 16;
        let host = mem.gpa_to_host(offset)?;
        unsafe {
            let addr = u64::from_le(std::ptr::read_unaligned(host as *const u64));
            let len = u32::from_le(std::ptr::read_unaligned(
                (host as *const u8).add(8) as *const u32
            ));
            let flags = u16::from_le(std::ptr::read_unaligned(
                (host as *const u8).add(12) as *const u16
            ));
            let next = u16::from_le(std::ptr::read_unaligned(
                (host as *const u8).add(14) as *const u16
            ));
            Some(VirtqDesc {
                addr,
                len,
                flags,
                next,
            })
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
    pub fn new(
        mem: GuestMemoryRef,
        desc_table_gpa: u64,
        avail_ring_gpa: u64,
        used_ring_gpa: u64,
        size: u16,
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
            desc_table_gpa,
            avail_ring_gpa,
            used_ring_gpa,
            size,
            next_avail,
            next_used,
            "virtqueue restored"
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
            desc_table_gpa,
            avail_ring_gpa,
            used_ring_gpa,
            size,
            next_avail,
            next_used,
            event_idx,
            "virtqueue restored"
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
            if visited >= self.size as u32 {
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
        let entry_gpa = self.avail_ring_gpa + 4 + (ring_index as u64) * 2;
        if let Some(ptr) = self.mem.gpa_to_host(entry_gpa) {
            unsafe { u16::from_le(std::ptr::read_unaligned(ptr as *const u16)) }
        } else {
            0
        }
    }

    /// Read `used_event` from the end of the available ring.
    fn read_used_event(&self) -> u16 {
        read_u16(&self.mem, self.avail_ring_gpa + 4 + (self.size as u64) * 2)
    }

    /// Write `avail_event` at the end of the used ring.
    fn write_avail_event(&self, idx: u16) {
        let event_gpa = self.used_ring_gpa + 4 + (self.size as u64) * 8;
        if let Some(ptr) = self.mem.gpa_to_host(event_gpa) {
            unsafe {
                std::ptr::write_unaligned(ptr as *mut u16, idx.to_le());
            }
        }
    }

    /// Write a used ring entry.
    fn write_used_ring(&self, ring_index: u16, id: u16, len: u32) {
        // used ring layout: flags (u16), idx (u16), ring[size] {id: u32, len: u32}
        let entry_gpa = self.used_ring_gpa + 4 + (ring_index as u64) * 8;
        if let Some(ptr) = self.mem.gpa_to_host(entry_gpa) {
            unsafe {
                std::ptr::write_unaligned(ptr as *mut u32, (id as u32).to_le());
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
mod tests {
    use super::super::memory::{GuestMemory, RAM_BASE};
    use super::*;

    // Helper: set up guest memory with a virtqueue at known offsets.
    // Returns (GuestMemory, desc_table_gpa, avail_ring_gpa, used_ring_gpa).
    fn setup_queue(size: u16) -> (GuestMemory, u64, u64, u64) {
        let mem_size = 1024 * 1024; // 1MB
        let mem = GuestMemory::new(mem_size).unwrap();

        // Place structures at offsets within guest RAM
        let desc_table_gpa = RAM_BASE;
        let avail_ring_gpa = RAM_BASE + (size as u64) * 16; // after descriptor table
        let used_ring_gpa = avail_ring_gpa + 6 + (size as u64) * 2; // after avail ring

        (mem, desc_table_gpa, avail_ring_gpa, used_ring_gpa)
    }

    // Helper: write a descriptor to guest memory
    fn write_desc(mem: &GuestMemory, desc_table_gpa: u64, index: u16, desc: &VirtqDesc) {
        let offset = (desc_table_gpa - RAM_BASE) + (index as u64) * 16;
        let mut data = [0u8; 16];
        data[0..8].copy_from_slice(&desc.addr.to_le_bytes());
        data[8..12].copy_from_slice(&desc.len.to_le_bytes());
        data[12..14].copy_from_slice(&desc.flags.to_le_bytes());
        data[14..16].copy_from_slice(&desc.next.to_le_bytes());
        mem.write_at(offset, &data).unwrap();
    }

    // Helper: write avail ring idx
    fn write_avail_idx(mem: &GuestMemory, avail_ring_gpa: u64, idx: u16) {
        let offset = (avail_ring_gpa - RAM_BASE) + 2; // skip flags
        mem.write_at(offset, &idx.to_le_bytes()).unwrap();
    }

    fn write_avail_flags(mem: &GuestMemory, avail_ring_gpa: u64, flags: u16) {
        let offset = avail_ring_gpa - RAM_BASE;
        mem.write_at(offset, &flags.to_le_bytes()).unwrap();
    }

    fn write_used_event(mem: &GuestMemory, avail_ring_gpa: u64, size: u16, idx: u16) {
        let offset = (avail_ring_gpa - RAM_BASE) + 4 + (size as u64) * 2;
        mem.write_at(offset, &idx.to_le_bytes()).unwrap();
    }

    fn read_avail_event(mem: &GuestMemory, used_ring_gpa: u64, size: u16) -> u16 {
        let offset = (used_ring_gpa - RAM_BASE) + 4 + (size as u64) * 8;
        let mut buf = [0u8; 2];
        mem.read_at(offset, &mut buf).unwrap();
        u16::from_le_bytes(buf)
    }

    // Helper: write avail ring entry
    fn write_avail_ring_entry(
        mem: &GuestMemory,
        avail_ring_gpa: u64,
        ring_index: u16,
        desc_idx: u16,
    ) {
        let offset = (avail_ring_gpa - RAM_BASE) + 4 + (ring_index as u64) * 2;
        mem.write_at(offset, &desc_idx.to_le_bytes()).unwrap();
    }

    // Helper: read used ring idx
    fn read_used_idx(mem: &GuestMemory, used_ring_gpa: u64) -> u16 {
        let offset = (used_ring_gpa - RAM_BASE) + 2;
        let mut buf = [0u8; 2];
        mem.read_at(offset, &mut buf).unwrap();
        u16::from_le_bytes(buf)
    }

    fn write_used_idx(mem: &GuestMemory, used_ring_gpa: u64, idx: u16) {
        let offset = (used_ring_gpa - RAM_BASE) + 2;
        mem.write_at(offset, &idx.to_le_bytes()).unwrap();
    }

    // Helper: read used ring entry
    fn read_used_entry(mem: &GuestMemory, used_ring_gpa: u64, ring_index: u16) -> (u32, u32) {
        let offset = (used_ring_gpa - RAM_BASE) + 4 + (ring_index as u64) * 8;
        let mut buf = [0u8; 8];
        mem.read_at(offset, &mut buf).unwrap();
        let id = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        (id, len)
    }

    // -----------------------------------------------------------------------
    // Pop tests
    // -----------------------------------------------------------------------

    #[test]
    fn pop_empty_queue() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);
        // avail_idx = 0 (no descriptors available)
        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        assert!(q.pop().is_none());
    }

    #[test]
    fn restored_queue_starts_after_used_entries() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);

        write_desc(
            &mem,
            desc_gpa,
            0,
            &VirtqDesc {
                addr: RAM_BASE + 0x1000,
                len: 256,
                flags: 0,
                next: 0,
            },
        );
        write_avail_ring_entry(&mem, avail_gpa, 0, 0);
        write_avail_idx(&mem, avail_gpa, 1);
        write_used_idx(&mem, used_gpa, 1);

        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        assert!(q.pop().is_none());
    }

    #[test]
    fn restored_queue_preserves_unprocessed_entries() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);

        write_desc(
            &mem,
            desc_gpa,
            1,
            &VirtqDesc {
                addr: RAM_BASE + 0x2000,
                len: 128,
                flags: 0,
                next: 0,
            },
        );
        write_avail_ring_entry(&mem, avail_gpa, 1, 1);
        write_avail_idx(&mem, avail_gpa, 2);
        write_used_idx(&mem, used_gpa, 1);

        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        let chain = q.pop().unwrap();
        assert_eq!(chain.head, 1);
        assert_eq!(chain.descriptors[0].addr, RAM_BASE + 0x2000);
        assert!(q.pop().is_none());
    }

    #[test]
    fn restored_queue_skips_pre_checkpoint_available_entries() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);

        write_desc(
            &mem,
            desc_gpa,
            1,
            &VirtqDesc {
                addr: RAM_BASE + 0x2000,
                len: 128,
                flags: 0,
                next: 0,
            },
        );
        write_avail_ring_entry(&mem, avail_gpa, 1, 1);
        write_avail_idx(&mem, avail_gpa, 2);
        write_used_idx(&mem, used_gpa, 1);

        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new_restored(memref, desc_gpa, avail_gpa, used_gpa, 16);

        assert!(q.pop().is_none());
    }

    #[test]
    fn pop_single_descriptor() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);

        // Write a single descriptor
        write_desc(
            &mem,
            desc_gpa,
            0,
            &VirtqDesc {
                addr: RAM_BASE + 0x1000,
                len: 256,
                flags: 0, // no NEXT, no WRITE
                next: 0,
            },
        );

        // Make it available
        write_avail_ring_entry(&mem, avail_gpa, 0, 0); // ring[0] = desc 0
        write_avail_idx(&mem, avail_gpa, 1); // 1 descriptor available

        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        let chain = q.pop().unwrap();
        assert_eq!(chain.head, 0);
        assert_eq!(chain.descriptors.len(), 1);
        assert_eq!(chain.descriptors[0].addr, RAM_BASE + 0x1000);
        assert_eq!(chain.descriptors[0].len, 256);
        assert!(!chain.descriptors[0].is_write_only());
        assert!(!chain.descriptors[0].has_next());

        // Second pop should return None
        assert!(q.pop().is_none());
    }

    #[test]
    fn pop_chained_descriptors() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);

        // Write 3 chained descriptors
        write_desc(
            &mem,
            desc_gpa,
            0,
            &VirtqDesc {
                addr: RAM_BASE + 0x1000,
                len: 16,
                flags: VRING_DESC_F_NEXT,
                next: 1,
            },
        );
        write_desc(
            &mem,
            desc_gpa,
            1,
            &VirtqDesc {
                addr: RAM_BASE + 0x2000,
                len: 1024,
                flags: VRING_DESC_F_NEXT | VRING_DESC_F_WRITE,
                next: 2,
            },
        );
        write_desc(
            &mem,
            desc_gpa,
            2,
            &VirtqDesc {
                addr: RAM_BASE + 0x3000,
                len: 1,
                flags: VRING_DESC_F_WRITE,
                next: 0,
            },
        );

        write_avail_ring_entry(&mem, avail_gpa, 0, 0);
        write_avail_idx(&mem, avail_gpa, 1);

        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        let chain = q.pop().unwrap();
        assert_eq!(chain.head, 0);
        assert_eq!(chain.descriptors.len(), 3);
        assert!(!chain.descriptors[0].is_write_only());
        assert!(chain.descriptors[1].is_write_only());
        assert!(chain.descriptors[2].is_write_only());
    }

    #[test]
    fn pop_multiple_chains() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);

        // Two independent single descriptors
        write_desc(
            &mem,
            desc_gpa,
            0,
            &VirtqDesc {
                addr: RAM_BASE + 0x1000,
                len: 100,
                flags: 0,
                next: 0,
            },
        );
        write_desc(
            &mem,
            desc_gpa,
            1,
            &VirtqDesc {
                addr: RAM_BASE + 0x2000,
                len: 200,
                flags: 0,
                next: 0,
            },
        );

        write_avail_ring_entry(&mem, avail_gpa, 0, 0);
        write_avail_ring_entry(&mem, avail_gpa, 1, 1);
        write_avail_idx(&mem, avail_gpa, 2);

        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        let chain1 = q.pop().unwrap();
        assert_eq!(chain1.head, 0);
        assert_eq!(chain1.descriptors[0].len, 100);

        let chain2 = q.pop().unwrap();
        assert_eq!(chain2.head, 1);
        assert_eq!(chain2.descriptors[0].len, 200);

        assert!(q.pop().is_none());
    }

    // -----------------------------------------------------------------------
    // Push used tests
    // -----------------------------------------------------------------------

    #[test]
    fn push_used_single() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);
        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        q.push_used(5, 1024);

        assert_eq!(read_used_idx(&mem, used_gpa), 1);
        let (id, len) = read_used_entry(&mem, used_gpa, 0);
        assert_eq!(id, 5);
        assert_eq!(len, 1024);
    }

    #[test]
    fn push_used_multiple() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);
        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        q.push_used(0, 100);
        q.push_used(3, 200);
        q.push_used(7, 300);

        assert_eq!(read_used_idx(&mem, used_gpa), 3);

        let (id, len) = read_used_entry(&mem, used_gpa, 0);
        assert_eq!((id, len), (0, 100));
        let (id, len) = read_used_entry(&mem, used_gpa, 1);
        assert_eq!((id, len), (3, 200));
        let (id, len) = read_used_entry(&mem, used_gpa, 2);
        assert_eq!((id, len), (7, 300));
    }

    #[test]
    fn push_used_deferred_publishes_idx_only_on_flush() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);
        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        q.push_used_deferred(0, 100);
        q.push_used_deferred(3, 200);

        assert_eq!(read_used_idx(&mem, used_gpa), 0);
        assert_eq!(read_used_entry(&mem, used_gpa, 0), (0, 100));
        assert_eq!(read_used_entry(&mem, used_gpa, 1), (3, 200));

        q.flush_used();

        assert_eq!(read_used_idx(&mem, used_gpa), 2);
    }

    #[test]
    fn prepare_kick_obeys_legacy_no_interrupt_flag() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);
        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        q.push_used_deferred(1, 64);
        q.flush_used();
        assert!(q.prepare_kick());

        write_avail_flags(&mem, avail_gpa, VRING_AVAIL_F_NO_INTERRUPT);
        q.push_used_deferred(2, 64);
        q.flush_used();
        assert!(!q.prepare_kick());
    }

    #[test]
    fn prepare_kick_obeys_event_idx_used_event() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);
        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new_with_event_idx(memref, desc_gpa, avail_gpa, used_gpa, 16, true);

        write_used_event(&mem, avail_gpa, 16, 4);
        q.push_used_deferred(1, 64);
        q.flush_used();
        assert!(!q.prepare_kick());

        q.push_used_deferred(2, 64);
        q.push_used_deferred(3, 64);
        q.push_used_deferred(4, 64);
        q.push_used_deferred(5, 64);
        q.flush_used();
        assert!(q.prepare_kick());
    }

    #[test]
    fn pop_or_enable_notification_arms_avail_event_when_empty() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);
        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new_with_event_idx(memref, desc_gpa, avail_gpa, used_gpa, 16, true);

        assert!(q.pop_or_enable_notification().is_none());

        assert_eq!(read_avail_event(&mem, used_gpa, 16), 0);
    }

    #[test]
    fn pop_or_enable_notification_drains_before_arming_avail_event() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);
        write_desc(
            &mem,
            desc_gpa,
            0,
            &VirtqDesc {
                addr: RAM_BASE + 0x1000,
                len: 64,
                flags: 0,
                next: 0,
            },
        );
        write_avail_ring_entry(&mem, avail_gpa, 0, 0);
        write_avail_idx(&mem, avail_gpa, 1);

        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new_with_event_idx(memref, desc_gpa, avail_gpa, used_gpa, 16, true);

        assert_eq!(q.pop_or_enable_notification().unwrap().head, 0);
        assert_eq!(read_avail_event(&mem, used_gpa, 16), 0);
        assert!(q.pop_or_enable_notification().is_none());
        assert_eq!(read_avail_event(&mem, used_gpa, 16), 1);
    }

    // -----------------------------------------------------------------------
    // Wrapping
    // -----------------------------------------------------------------------

    #[test]
    fn avail_ring_wraps() {
        let queue_size = 4u16;
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(queue_size);

        // Fill all 4 slots
        for i in 0..queue_size {
            write_desc(
                &mem,
                desc_gpa,
                i,
                &VirtqDesc {
                    addr: RAM_BASE + (i as u64) * 0x1000,
                    len: 64,
                    flags: 0,
                    next: 0,
                },
            );
            write_avail_ring_entry(&mem, avail_gpa, i, i);
        }
        write_avail_idx(&mem, avail_gpa, 4);

        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, queue_size);

        // Pop all 4
        for _ in 0..4 {
            assert!(q.pop().is_some());
        }
        assert!(q.pop().is_none());
    }

    // -----------------------------------------------------------------------
    // Cycle detection
    // -----------------------------------------------------------------------

    #[test]
    fn cycle_in_descriptor_chain_terminates() {
        let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);

        // Create a cycle: desc 0 -> desc 1 -> desc 0
        write_desc(
            &mem,
            desc_gpa,
            0,
            &VirtqDesc {
                addr: RAM_BASE + 0x1000,
                len: 64,
                flags: VRING_DESC_F_NEXT,
                next: 1,
            },
        );
        write_desc(
            &mem,
            desc_gpa,
            1,
            &VirtqDesc {
                addr: RAM_BASE + 0x2000,
                len: 64,
                flags: VRING_DESC_F_NEXT,
                next: 0,
            },
        );

        write_avail_ring_entry(&mem, avail_gpa, 0, 0);
        write_avail_idx(&mem, avail_gpa, 1);

        let memref = mem.clone_ref(RAM_BASE);
        let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 16);

        // Should terminate (cycle detection kicks in at queue_size iterations)
        let chain = q.pop().unwrap();
        assert!(chain.descriptors.len() <= 16);
    }

    // -----------------------------------------------------------------------
    // Descriptor flags
    // -----------------------------------------------------------------------

    #[test]
    fn descriptor_flag_helpers() {
        let read_only = VirtqDesc {
            addr: 0,
            len: 0,
            flags: 0,
            next: 0,
        };
        assert!(!read_only.is_write_only());
        assert!(!read_only.has_next());

        let write_only = VirtqDesc {
            addr: 0,
            len: 0,
            flags: VRING_DESC_F_WRITE,
            next: 0,
        };
        assert!(write_only.is_write_only());

        let chained = VirtqDesc {
            addr: 0,
            len: 0,
            flags: VRING_DESC_F_NEXT,
            next: 5,
        };
        assert!(chained.has_next());

        let both = VirtqDesc {
            addr: 0,
            len: 0,
            flags: VRING_DESC_F_NEXT | VRING_DESC_F_WRITE,
            next: 3,
        };
        assert!(both.is_write_only());
        assert!(both.has_next());
    }
}
