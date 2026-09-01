use super::super::memory::{GuestMemory, RAM_BASE};
use super::*;

// Helper: set up guest memory with a virtqueue at known offsets.
// Returns (GuestMemory, desc_table_gpa, avail_ring_gpa, used_ring_gpa).
fn setup_queue(size: u16) -> (GuestMemory, u64, u64, u64) {
    let mem_size = 1024 * 1024; // 1MB
    let mem = GuestMemory::new(mem_size).unwrap();

    // Place structures at offsets within guest RAM
    let desc_table_gpa = RAM_BASE;
    let avail_ring_gpa = RAM_BASE + u64::from(size) * 16; // after descriptor table
    let used_ring_gpa = avail_ring_gpa + 6 + u64::from(size) * 2; // after avail ring

    (mem, desc_table_gpa, avail_ring_gpa, used_ring_gpa)
}

// Helper: write a descriptor to guest memory
fn write_desc(mem: &GuestMemory, desc_table_gpa: u64, index: u16, desc: &VirtqDesc) {
    let offset = (desc_table_gpa - RAM_BASE) + u64::from(index) * 16;
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
    let offset = (avail_ring_gpa - RAM_BASE) + 4 + u64::from(size) * 2;
    mem.write_at(offset, &idx.to_le_bytes()).unwrap();
}

fn read_avail_event(mem: &GuestMemory, used_ring_gpa: u64, size: u16) -> u16 {
    let offset = (used_ring_gpa - RAM_BASE) + 4 + u64::from(size) * 8;
    let mut buf = [0u8; 2];
    mem.read_at(offset, &mut buf).unwrap();
    u16::from_le_bytes(buf)
}

// Helper: write avail ring entry
fn write_avail_ring_entry(mem: &GuestMemory, avail_ring_gpa: u64, ring_index: u16, desc_idx: u16) {
    let offset = (avail_ring_gpa - RAM_BASE) + 4 + u64::from(ring_index) * 2;
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
    let offset = (used_ring_gpa - RAM_BASE) + 4 + u64::from(ring_index) * 8;
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
fn pop_zero_size_queue_does_not_panic() {
    // A guest can drive DRIVER_OK with QUEUE_NUM=0 on the cold path, which is
    // not clamped to the device max. `next_avail % size` would divide by zero
    // and panic the host process (a guest-triggerable DoS). A zero-size queue
    // must report empty, never panic.
    let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);
    // Make the avail index non-zero so pop() gets past the ring-empty check and
    // reaches the `% size` computation.
    write_avail_idx(&mem, avail_gpa, 1);
    let memref = mem.clone_ref(RAM_BASE);
    let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 0);
    assert!(q.pop().is_none());
}

#[test]
fn push_used_zero_size_queue_does_not_panic() {
    let (mem, desc_gpa, avail_gpa, used_gpa) = setup_queue(16);
    let memref = mem.clone_ref(RAM_BASE);
    let mut q = VirtQueue::new(memref, desc_gpa, avail_gpa, used_gpa, 0);
    // Must be a no-op rather than dividing by zero in `next_used % size`.
    q.push_used(0, 0);
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
                addr: RAM_BASE + u64::from(i) * 0x1000,
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
