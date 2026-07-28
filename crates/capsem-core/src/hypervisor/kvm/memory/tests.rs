use super::*;

// -----------------------------------------------------------------------
// aarch64 address map constants
// -----------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
#[test]
fn gic_below_ram() {
    const {
        assert!(GIC_DIST_BASE + GIC_DIST_SIZE <= RAM_BASE);
        assert!(GIC_REDIST_BASE < RAM_BASE);
    }
}

#[cfg(target_arch = "aarch64")]
#[test]
fn virtio_mmio_below_ram() {
    let max_addr = virtio_mmio_addr(VIRTIO_MMIO_MAX_DEVICES - 1) + VIRTIO_MMIO_SIZE;
    assert!(max_addr <= RAM_BASE, "virtio MMIO region overlaps RAM");
}

#[cfg(target_arch = "aarch64")]
#[test]
fn gic_does_not_overlap_virtio() {
    let gic_end = GIC_REDIST_BASE + GIC_REDIST_PER_CPU * 8; // max 8 CPUs
    assert!(
        gic_end <= VIRTIO_MMIO_BASE
            || GIC_DIST_BASE
                >= VIRTIO_MMIO_BASE + VIRTIO_MMIO_SIZE * VIRTIO_MMIO_MAX_DEVICES as u64,
        "GIC and virtio MMIO regions overlap"
    );
}

#[cfg(target_arch = "aarch64")]
#[test]
fn virtio_mmio_addr_sequential() {
    assert_eq!(virtio_mmio_addr(0), VIRTIO_MMIO_BASE);
    assert_eq!(virtio_mmio_addr(1), VIRTIO_MMIO_BASE + 0x200);
    assert_eq!(virtio_mmio_addr(2), VIRTIO_MMIO_BASE + 0x400);
}

#[cfg(target_arch = "aarch64")]
#[test]
fn virtio_mmio_irq_sequential() {
    assert_eq!(virtio_mmio_irq(0), 48);
    assert_eq!(virtio_mmio_irq(1), 49);
    assert_eq!(virtio_mmio_irq(2), 50);
}

#[cfg(target_arch = "aarch64")]
#[test]
fn virtio_slots_dont_overlap() {
    for i in 0..VIRTIO_MMIO_MAX_DEVICES {
        for j in (i + 1)..VIRTIO_MMIO_MAX_DEVICES {
            let a_start = virtio_mmio_addr(i);
            let a_end = a_start + VIRTIO_MMIO_SIZE;
            let b_start = virtio_mmio_addr(j);
            assert!(a_end <= b_start, "slot {i} overlaps slot {j}");
        }
    }
}

// -----------------------------------------------------------------------
// Page alignment helpers
// -----------------------------------------------------------------------

#[test]
fn page_align_up_already_aligned() {
    assert_eq!(page_align_up(4096), 4096);
    assert_eq!(page_align_up(0), 0);
    assert_eq!(page_align_up(8192), 8192);
}

#[test]
fn page_align_up_not_aligned() {
    assert_eq!(page_align_up(1), 4096);
    assert_eq!(page_align_up(4095), 4096);
    assert_eq!(page_align_up(4097), 8192);
}

#[test]
fn page_align_down_already_aligned() {
    assert_eq!(page_align_down(4096), 4096);
    assert_eq!(page_align_down(0), 0);
}

#[test]
fn page_align_down_not_aligned() {
    assert_eq!(page_align_down(4095), 0);
    assert_eq!(page_align_down(4097), 4096);
    assert_eq!(page_align_down(8191), 4096);
}

// -----------------------------------------------------------------------
// GuestMemory
// -----------------------------------------------------------------------

#[test]
fn guest_memory_new_valid() {
    let mem = GuestMemory::new(4096).unwrap();
    assert_eq!(mem.size(), 4096);
    assert!(!mem.as_ptr().is_null());
}

#[test]
fn guest_memory_new_zero_fails() {
    assert!(GuestMemory::new(0).is_err());
}

#[test]
fn guest_memory_new_unaligned_fails() {
    assert!(GuestMemory::new(4095).is_err());
    assert!(GuestMemory::new(100).is_err());
}

#[test]
fn guest_memory_write_and_read() {
    let mem = GuestMemory::new(4096).unwrap();
    let data = b"hello guest memory";
    mem.write_at(0, data).unwrap();

    let mut buf = vec![0u8; data.len()];
    mem.read_at(0, &mut buf).unwrap();
    assert_eq!(buf, data);
}

#[test]
fn guest_memory_write_at_offset() {
    let mem = GuestMemory::new(4096).unwrap();
    let data = b"offset";
    mem.write_at(100, data).unwrap();

    let mut buf = vec![0u8; data.len()];
    mem.read_at(100, &mut buf).unwrap();
    assert_eq!(buf, data);
}

#[test]
fn guest_memory_write_out_of_bounds() {
    let mem = GuestMemory::new(4096).unwrap();
    let data = vec![0u8; 4097];
    assert!(mem.write_at(0, &data).is_err());
}

#[test]
fn guest_memory_write_at_end() {
    let mem = GuestMemory::new(4096).unwrap();
    // Writing 1 byte at offset 4095 should succeed (last byte)
    mem.write_at(4095, &[0xAB]).unwrap();
    let mut buf = [0u8];
    mem.read_at(4095, &mut buf).unwrap();
    assert_eq!(buf[0], 0xAB);
}

#[test]
fn guest_memory_write_past_end() {
    let mem = GuestMemory::new(4096).unwrap();
    assert!(mem.write_at(4096, &[0]).is_err());
}

#[test]
fn guest_memory_write_offset_overflow_fails() {
    let mem = GuestMemory::new(4096).unwrap();
    assert!(mem.write_at(u64::MAX, &[0]).is_err());
}

#[test]
fn guest_memory_read_offset_overflow_fails() {
    let mem = GuestMemory::new(4096).unwrap();
    let mut buf = [0u8; 1];
    assert!(mem.read_at(u64::MAX, &mut buf).is_err());
}

#[test]
fn guest_memory_read_out_of_bounds() {
    let mem = GuestMemory::new(4096).unwrap();
    let mut buf = vec![0u8; 4097];
    assert!(mem.read_at(0, &mut buf).is_err());
}

#[test]
fn guest_memory_is_zero_initialized() {
    let mem = GuestMemory::new(4096).unwrap();
    let mut buf = vec![0xFFu8; 4096];
    mem.read_at(0, &mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0),
        "memory should be zero-initialized"
    );
}

#[test]
fn guest_memory_large_allocation() {
    // 256MB -- should work as sparse mmap
    let size = 256 * 1024 * 1024u64;
    let mem = GuestMemory::new(size).unwrap();
    assert_eq!(mem.size(), size);

    // Write at the end
    mem.write_at(size - 8, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    let mut buf = [0u8; 8];
    mem.read_at(size - 8, &mut buf).unwrap();
    assert_eq!(buf, [1, 2, 3, 4, 5, 6, 7, 8]);
}

// -----------------------------------------------------------------------
// GuestMemoryRef
// -----------------------------------------------------------------------

#[test]
fn guest_memory_ref_gpa_to_host() {
    let mem = GuestMemory::new(4096).unwrap();
    let memref = mem.clone_ref(RAM_BASE);

    // Address within RAM region
    let ptr = memref.gpa_to_host(RAM_BASE);
    assert!(ptr.is_some());

    // Address before RAM base
    let before_ram_base = RAM_BASE.checked_sub(1).unwrap_or(u64::MAX);
    let ptr = memref.gpa_to_host(before_ram_base);
    assert!(ptr.is_none());

    // Address past end
    let ptr = memref.gpa_to_host(RAM_BASE + 4096);
    assert!(ptr.is_none());
}

#[test]
fn guest_memory_ref_gpa_range_to_host_validates_full_range() {
    let mem = GuestMemory::new(4096).unwrap();
    let memref = mem.clone_ref(RAM_BASE);

    assert!(memref.gpa_range_to_host(RAM_BASE + 4095, 1).is_some());
    assert!(memref.gpa_range_to_host(RAM_BASE + 4095, 2).is_none());
    assert!(memref.gpa_range_to_host(RAM_BASE + 4096, 0).is_none());
    assert!(memref.gpa_range_to_host(u64::MAX - 1, 8).is_none());
}

#[test]
fn guest_memory_ref_write_read() {
    let mem = GuestMemory::new(4096).unwrap();
    let memref = mem.clone_ref(RAM_BASE);

    memref.write_at(0, b"via ref").unwrap();
    let mut buf = vec![0u8; 7];
    memref.read_at(0, &mut buf).unwrap();
    assert_eq!(buf, b"via ref");
}

#[test]
fn guest_memory_ref_write_offset_overflow_fails() {
    let mem = GuestMemory::new(4096).unwrap();
    let memref = mem.clone_ref(RAM_BASE);
    assert!(memref.write_at(u64::MAX, &[0]).is_err());
}

#[test]
fn guest_memory_ref_read_offset_overflow_fails() {
    let mem = GuestMemory::new(4096).unwrap();
    let memref = mem.clone_ref(RAM_BASE);
    let mut buf = [0u8; 1];
    assert!(memref.read_at(u64::MAX, &mut buf).is_err());
}

#[test]
fn guest_memory_ref_shares_underlying_memory() {
    let mem = GuestMemory::new(4096).unwrap();
    let memref = mem.clone_ref(RAM_BASE);

    // Write via original
    mem.write_at(0, b"shared").unwrap();
    // Read via ref
    let mut buf = vec![0u8; 6];
    memref.read_at(0, &mut buf).unwrap();
    assert_eq!(buf, b"shared");
}

// -----------------------------------------------------------------------
// Kernel/initrd placement calculations (aarch64)
// -----------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
#[test]
fn kernel_loads_at_correct_offset() {
    let kernel_addr = RAM_BASE + KERNEL_TEXT_OFFSET;
    assert_eq!(kernel_addr, 0x4008_0000);
}

#[cfg(target_arch = "aarch64")]
#[test]
fn initrd_at_end_of_ram_page_aligned() {
    let ram_size: u64 = 4 * 1024 * 1024 * 1024; // 4GB
    let initrd_size: u64 = 50 * 1024 * 1024; // 50MB
    let ram_end = RAM_BASE + ram_size;

    let initrd_start = page_align_down(ram_end - initrd_size);
    assert!(initrd_start.is_multiple_of(PAGE_SIZE));
    assert!(initrd_start + initrd_size <= ram_end);
    assert!(initrd_start > RAM_BASE + KERNEL_TEXT_OFFSET); // doesn't overlap kernel region
}

#[cfg(target_arch = "aarch64")]
#[test]
fn fdt_after_kernel_page_aligned() {
    let kernel_size: u64 = 30 * 1024 * 1024; // 30MB
    let kernel_end = RAM_BASE + KERNEL_TEXT_OFFSET + kernel_size;
    let fdt_start = page_align_up(kernel_end);

    assert!(fdt_start.is_multiple_of(PAGE_SIZE));
    assert!(fdt_start >= kernel_end);
    // FDT must be within 512MB of kernel entry
    assert!(fdt_start - (RAM_BASE + KERNEL_TEXT_OFFSET) < 512 * 1024 * 1024);
}

#[cfg(target_arch = "aarch64")]
#[test]
fn kernel_initrd_fdt_fit_in_ram() {
    let ram_size: u64 = 512 * 1024 * 1024; // 512MB (minimum practical)
    let kernel_size: u64 = 30 * 1024 * 1024; // 30MB
    let initrd_size: u64 = 100 * 1024 * 1024; // 100MB
    let fdt_size: u64 = 1024 * 1024; // 1MB (generous)

    let ram_end = RAM_BASE + ram_size;
    let kernel_end = RAM_BASE + KERNEL_TEXT_OFFSET + kernel_size;
    let fdt_end = page_align_up(kernel_end) + fdt_size;
    let initrd_start = page_align_down(ram_end - initrd_size);

    assert!(
        fdt_end <= initrd_start,
        "FDT (end {fdt_end:#x}) overlaps initrd (start {initrd_start:#x})"
    );
}

// -----------------------------------------------------------------------
// x86_64 memory layout
// -----------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
#[test]
#[allow(clippy::assertions_on_constants)]
fn x86_64_kernel_above_legacy_hole() {
    assert!(KERNEL_LOAD_ADDR >= HIGH_MEM_START);
}

#[cfg(target_arch = "x86_64")]
#[test]
#[allow(clippy::assertions_on_constants)]
fn x86_64_boot_structs_below_ebda() {
    assert!(BOOT_PARAMS_ADDR + 4096 <= EBDA_START);
    assert!(GDT_ADDR + 24 <= EBDA_START);
    assert!(PML4_ADDR + PAGE_SIZE <= EBDA_START);
    assert!(PDPT_ADDR + PAGE_SIZE <= EBDA_START);
    assert!(PD_ADDR + PAGE_SIZE <= EBDA_START);
}

#[cfg(target_arch = "x86_64")]
#[test]
#[allow(clippy::assertions_on_constants)]
fn x86_64_boot_structs_no_overlap() {
    // GDT: 0x500..0x518 (24 bytes)
    // BOOT_PARAMS: 0x7000..0x8000 (4096 bytes)
    // PML4: 0x9000..0xA000
    // PDPT: 0xA000..0xB000
    // PD: 0xB000..0xC000
    // CMDLINE: 0x20000..0x21000
    assert!(GDT_ADDR + 24 <= BOOT_PARAMS_ADDR);
    assert!(BOOT_PARAMS_ADDR + PAGE_SIZE <= PML4_ADDR);
    assert!(PML4_ADDR + PAGE_SIZE == PDPT_ADDR);
    assert!(PDPT_ADDR + PAGE_SIZE == PD_ADDR);
    assert!(PD_ADDR + PAGE_SIZE <= CMDLINE_ADDR);
    assert!(CMDLINE_ADDR + CMDLINE_MAX_SIZE <= KERNEL_LOAD_ADDR);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn x86_64_e820_map() {
    let ram_size = 512 * 1024 * 1024u64; // 512 MiB
    let entries = build_e820_map(ram_size);
    assert_eq!(entries.len(), 3);

    // Low RAM: 0..640K
    assert_eq!(entries[0].addr, 0);
    assert_eq!(entries[0].size, EBDA_START);
    assert_eq!(entries[0].type_, E820_RAM);

    // ISA hole: 640K..1M
    assert_eq!(entries[1].addr, EBDA_START);
    assert_eq!(entries[1].type_, E820_RESERVED);

    // High RAM: 1M..512M
    assert_eq!(entries[2].addr, HIGH_MEM_START);
    assert_eq!(entries[2].size, ram_size - HIGH_MEM_START);
    assert_eq!(entries[2].type_, E820_RAM);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn x86_64_e820_map_reserves_pci_hole_above_3gb() {
    let ram_size = 8 * 1024 * 1024 * 1024u64;
    let entries = build_e820_map(ram_size);
    assert_eq!(entries.len(), 5);
    assert_eq!(entries[2].addr, HIGH_MEM_START);
    assert_eq!(entries[2].size, PCI_HOLE_START - HIGH_MEM_START);
    assert_eq!(entries[2].type_, E820_RAM);
    assert_eq!(entries[3].addr, PCI_HOLE_START);
    assert_eq!(entries[3].size, PCI_HOLE_SIZE);
    assert_eq!(entries[3].type_, E820_RESERVED);
    assert_eq!(entries[4].addr, PCI_HOLE_END);
    assert_eq!(entries[4].size, ram_size - PCI_HOLE_START);
    assert_eq!(entries[4].type_, E820_RAM);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn x86_64_kvm_memory_regions_split_around_pci_hole() {
    let regions = kvm_memory_regions(8 * 1024 * 1024 * 1024u64);
    assert_eq!(
        regions,
        vec![
            KvmMemoryRegion {
                slot: 0,
                guest_phys_addr: 0,
                memory_size: PCI_HOLE_START,
                host_offset: 0,
            },
            KvmMemoryRegion {
                slot: 1,
                guest_phys_addr: PCI_HOLE_END,
                memory_size: 5 * 1024 * 1024 * 1024u64,
                host_offset: PCI_HOLE_START,
            },
        ]
    );
    assert_eq!(
        guest_phys_end(8 * 1024 * 1024 * 1024u64),
        9 * 1024 * 1024 * 1024u64
    );
    assert_eq!(
        gpa_to_ram_offset(PCI_HOLE_START - 1, 8 * 1024 * 1024 * 1024u64),
        Some(PCI_HOLE_START - 1)
    );
    assert_eq!(
        gpa_to_ram_offset(PCI_HOLE_START, 8 * 1024 * 1024 * 1024u64),
        None
    );
    assert_eq!(
        gpa_to_ram_offset(PCI_HOLE_END, 8 * 1024 * 1024 * 1024u64),
        Some(PCI_HOLE_START)
    );
}

#[cfg(target_arch = "x86_64")]
#[test]
fn x86_64_virtio_mmio_sequential() {
    assert_eq!(virtio_mmio_addr(0), VIRTIO_MMIO_BASE);
    assert_eq!(virtio_mmio_addr(1), VIRTIO_MMIO_BASE + 0x200);
    assert_eq!(virtio_mmio_irq(0), 5);
    assert_eq!(virtio_mmio_irq(1), 6);
}

#[cfg(target_arch = "x86_64")]
#[test]
#[allow(clippy::assertions_on_constants)]
fn x86_64_virtio_mmio_in_pci_hole() {
    let window_end = VIRTIO_MMIO_BASE + VIRTIO_MMIO_SIZE * VIRTIO_MMIO_MAX_DEVICES as u64;
    assert!(
        VIRTIO_MMIO_BASE >= PCI_HOLE_START,
        "Virtio MMIO base {VIRTIO_MMIO_BASE:#x} must be inside the PCI hole"
    );
    assert!(
        window_end <= PCI_HOLE_END,
        "Virtio MMIO window {VIRTIO_MMIO_BASE:#x}..{window_end:#x} must fit inside the PCI hole"
    );
}

#[cfg(target_arch = "x86_64")]
#[test]
#[allow(clippy::assertions_on_constants)]
fn x86_64_irq_base_above_legacy() {
    assert!(
        VIRTIO_MMIO_IRQ_BASE > 4,
        "must not conflict with ISA IRQs 0-4"
    );
}

#[cfg(target_arch = "aarch64")]
#[test]
fn aarch64_gic_spi_range_valid() {
    const {
        assert!(
            VIRTIO_MMIO_IRQ_BASE >= 32,
            "virtio IRQs must be in GIC SPI range (>=32)"
        );
    }
    let max_irq = VIRTIO_MMIO_IRQ_BASE + VIRTIO_MMIO_MAX_DEVICES;
    assert!(
        max_irq < 1020,
        "virtio IRQs must stay within GIC SPI range (<1020)"
    );
}
