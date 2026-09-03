use super::*;

// A descriptor whose [addr, addr+len) range escapes guest RAM must be rejected
// wholesale, never read or written past the mmap.
const VRING_DESC_F_WRITE: u16 = 2;

fn desc(addr: u64, len: u32, flags: u16) -> VirtqDesc {
    VirtqDesc {
        addr,
        len,
        flags,
        next: 0,
    }
}

fn chain(descriptors: Vec<VirtqDesc>) -> DescriptorChain {
    DescriptorChain { head: 0, descriptors }
}

#[test]
fn gather_readable_skips_descriptor_spilling_past_ram() {
    let mem = GuestMemory::new(2 * 0x1000).unwrap();
    let memref = mem.clone_ref(RAM_BASE);
    mem.write_at(0x100, &[0x7Au8; 16]).unwrap();
    let ram = mem.size();
    let c = chain(vec![desc(RAM_BASE + 0x100, 16, 0), desc(RAM_BASE + ram - 8, 4096, 0)]);

    let buf = gather_readable(&memref, &c).expect("gather within MAX_GATHER_SIZE");

    assert_eq!(buf, vec![0x7Au8; 16]);
}

#[test]
fn write_response_rejects_descriptor_spilling_past_ram() {
    let mem = GuestMemory::new(2 * 0x1000).unwrap();
    let memref = mem.clone_ref(RAM_BASE);
    let ram = mem.size();
    let c = chain(vec![desc(RAM_BASE + ram - 8, 4096, VRING_DESC_F_WRITE)]);
    let data = vec![0xE5u8; 512];

    let written = write_response(&memref, &c, &data);

    assert_eq!(written, 0, "OOB write-only descriptor must receive nothing");
    let mut back = [0u8; 8];
    mem.read_at(ram - 8, &mut back).unwrap();
    assert_eq!(back, [0u8; 8]);
}

#[test]
fn write_response_writes_into_valid_descriptor() {
    let mem = GuestMemory::new(2 * 0x1000).unwrap();
    let memref = mem.clone_ref(RAM_BASE);
    let c = chain(vec![desc(RAM_BASE + 0x200, 64, VRING_DESC_F_WRITE)]);
    let data = vec![0x3Cu8; 40];

    let written = write_response(&memref, &c, &data);

    assert_eq!(written, 40);
    let mut back = [0u8; 40];
    mem.read_at(0x200, &mut back).unwrap();
    assert_eq!(back, [0x3Cu8; 40]);
}
