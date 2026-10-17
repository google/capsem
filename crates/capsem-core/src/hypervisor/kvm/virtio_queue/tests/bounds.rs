use super::*;

#[cfg(target_arch = "x86_64")]
#[test]
fn descriptor_metadata_must_fit_one_complete_guest_ram_span() {
    use crate::hypervisor::kvm::memory::{PCI_HOLE_END, PCI_HOLE_START};
    // The host mapping continues past the PCI hole's guest address. This
    // tests rejection without dereferencing unmapped host memory on RED.
    let mem = GuestMemory::new(PCI_HOLE_START + 4096).unwrap();
    let reader = mem.clone_ref(RAM_BASE);
    for table in [PCI_HOLE_START - 8, PCI_HOLE_END + 4096 - 8, u64::MAX - 8] {
        assert!(VirtqDesc::read_from(&reader, table, 0).is_none(), "{table:#x}");
    }
    assert!(VirtqDesc::read_from(&reader, u64::MAX - 8, 1).is_none());
}

#[cfg(target_arch = "x86_64")]
#[test]
fn ring_cells_crossing_a_guest_ram_span_are_neither_read_nor_written() {
    use crate::hypervisor::kvm::memory::PCI_HOLE_START;
    let mem = GuestMemory::new(PCI_HOLE_START + 4096).unwrap();
    let boundary = PCI_HOLE_START - 1;
    mem.write_at(boundary, &[0x7b, 0x02]).unwrap();
    assert_eq!(read_u16(&mem.clone_ref(RAM_BASE), boundary), 0);
    let queue = VirtQueue::new(mem.clone_ref(RAM_BASE), 0, 0, boundary - 2, 1);
    queue.write_used_idx(0xabcd);
    let mut after = [0; 2];
    mem.read_at(boundary, &mut after).unwrap();
    assert_eq!(after, [0x7b, 0x02]);
    let queue = VirtQueue::new(mem.clone_ref(RAM_BASE), 0, 0, boundary - 12, 1);
    queue.write_avail_event(0xabcd);
    mem.read_at(boundary, &mut after).unwrap();
    assert_eq!(after, [0x7b, 0x02]);
    let boundary = PCI_HOLE_START - 7;
    mem.write_at(boundary, &[0x7b; 8]).unwrap();
    let queue = VirtQueue::new(mem.clone_ref(RAM_BASE), 0, 0, boundary - 4, 1);
    queue.write_used_ring(0, 9, 1024);
    let mut after = [0; 8];
    mem.read_at(boundary, &mut after).unwrap();
    assert_eq!(after, [0x7b; 8]);
}
