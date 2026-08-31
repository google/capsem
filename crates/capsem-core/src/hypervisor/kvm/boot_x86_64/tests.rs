use super::*;

#[test]
fn gdt_entries_correct_size() {
    // 4 entries * 8 bytes = 32 bytes
    let mem = GuestMemory::new(4096 * 16).unwrap();
    write_gdt(&mem).unwrap();
    let mut buf = [0u8; 32];
    mem.read_at(GDT_ADDR - RAM_BASE, &mut buf).unwrap();
    // Null entry should be zero
    assert_eq!(&buf[..8], &[0u8; 8]);
    // Entry 1 intentionally unused so Linux boot CS can be 0x10.
    assert_eq!(&buf[8..16], &[0u8; 8]);
    assert_ne!(&buf[16..24], &[0u8; 8]);
    assert_ne!(&buf[24..32], &[0u8; 8]);
}

#[test]
fn page_tables_identity_mapped() {
    let mem = GuestMemory::new(4096 * 16).unwrap();
    write_page_tables(&mem, 4 * 1024 * 1024 * 1024).unwrap();

    // PML4[0] should point to PDPT with present+writable
    let mut buf = [0u8; 8];
    mem.read_at(PML4_ADDR - RAM_BASE, &mut buf).unwrap();
    let pml4_entry = u64::from_le_bytes(buf);
    assert_eq!(pml4_entry & !0xFFF, PDPT_ADDR);
    assert_eq!(pml4_entry & 0x3, 0x3); // present + writable

    // PD[0] should map 0..2MiB with huge page
    mem.read_at(PD_ADDR - RAM_BASE, &mut buf).unwrap();
    let pd0 = u64::from_le_bytes(buf);
    assert_eq!(pd0 & !(0x1FFFFF), 0); // maps physical 0
    assert_ne!(pd0 & 0x80, 0); // PS bit (huge page)
}

#[test]
fn write_page_tables_exact_gb_boundaries() {
    let mem = GuestMemory::new(1024 * 1024).unwrap(); // 1MB for structs

    // Test exactly 1GB
    write_page_tables(&mem, 1024 * 1024 * 1024).unwrap();
    let mut buf = [0u8; 8];
    mem.read_at(PDPT_ADDR - RAM_BASE, &mut buf).unwrap();
    assert_ne!(u64::from_le_bytes(buf), 0, "PDPT[0] should exist for 1GB");

    // PDPT[1] should technically be mapped because we round up our gb_count,
    // or if we do exact division it might be empty. Let's just ensure it doesn't panic.

    // Test exactly 2GB
    write_page_tables(&mem, 2 * 1024 * 1024 * 1024).unwrap();
    mem.read_at(PDPT_ADDR - RAM_BASE + 8, &mut buf).unwrap();
    assert_ne!(u64::from_le_bytes(buf), 0, "PDPT[1] should exist for 2GB");
}

#[test]
fn boot_params_sets_cmdline() {
    let mem = GuestMemory::new(4096 * 256).unwrap();
    let cmdline = "console=ttyS0 root=/dev/vda ro";
    let e820 = memory::build_e820_map(256 * 4096);
    write_boot_params(&mem, cmdline, None, &e820, &[]).unwrap();

    // Check cmdline was written
    let mut buf = vec![0u8; cmdline.len()];
    mem.read_at(CMDLINE_ADDR - RAM_BASE, &mut buf).unwrap();
    assert_eq!(&buf, cmdline.as_bytes());

    // Check cmd_line_ptr in boot_params
    let mut ptr_buf = [0u8; 4];
    mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x228, &mut ptr_buf).unwrap();
    assert_eq!(u32::from_le_bytes(ptr_buf), CMDLINE_ADDR as u32);
}

#[test]
fn boot_params_sets_initrd() {
    let mem = GuestMemory::new(4096 * 256).unwrap();
    let initrd = InitrdLoadInfo {
        addr: 0x80_0000,
        size: 1024 * 1024,
    };
    let e820 = memory::build_e820_map(256 * 4096);
    write_boot_params(&mem, "test", Some(&initrd), &e820, &[]).unwrap();

    // Check ramdisk_image
    let mut buf = [0u8; 4];
    mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x218, &mut buf).unwrap();
    assert_eq!(u32::from_le_bytes(buf), 0x80_0000);
    // Check ramdisk_size
    mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x21C, &mut buf).unwrap();
    assert_eq!(u32::from_le_bytes(buf), 1024 * 1024);
}

#[test]
fn write_boot_params_preserves_setup_header() {
    let mem = GuestMemory::new(4096 * 256).unwrap();
    let mut fake_header = vec![0u8; 0x2b9 - 0x1f1];
    fake_header[0] = 0xAA;
    let last_idx = fake_header.len() - 1;
    fake_header[last_idx] = 0xBB;

    let e820 = memory::build_e820_map(256 * 4096);
    write_boot_params(&mem, "test", None, &e820, &fake_header).unwrap();

    let mut buf = [0u8; 1];
    mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x1f1, &mut buf).unwrap();
    assert_eq!(buf[0], 0xAA, "First byte of setup_header not preserved");

    mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x2b8, &mut buf).unwrap();
    assert_eq!(buf[0], 0xBB, "Last byte of setup_header not preserved");
}

#[test]
fn write_boot_params_sets_loader_and_flags() {
    let mem = GuestMemory::new(4096 * 256).unwrap();
    let e820 = memory::build_e820_map(256 * 4096);
    write_boot_params(&mem, "test", None, &e820, &[]).unwrap();

    let mut buf = [0u8; 1];
    mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x210, &mut buf).unwrap();
    assert_eq!(buf[0], 0xFF, "type_of_loader must be 0xFF");

    mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x211, &mut buf).unwrap();
    assert_eq!(buf[0], 0x81, "loadflags must be 0x81 (LOADED_HIGH | CAN_USE_HEAP)");
}

#[test]
fn acpi_tables_advertise_all_vcpus_in_madt() {
    let mem = GuestMemory::new(1024 * 1024).unwrap();
    write_acpi_tables(&mem, 4).unwrap();

    let mut rsdp = [0u8; 20];
    mem.read_at(memory::ACPI_RSDP_ADDR - RAM_BASE, &mut rsdp).unwrap();
    assert_eq!(&rsdp[0..8], b"RSD PTR ");
    assert_eq!(checksum(&rsdp), 0);
    assert_eq!(
        u32::from_le_bytes(rsdp[16..20].try_into().unwrap()),
        memory::ACPI_RSDT_ADDR as u32
    );

    let mut ebda_segment = [0u8; 2];
    mem.read_at(memory::BDA_EBDA_SEGMENT_ADDR - RAM_BASE, &mut ebda_segment)
        .unwrap();
    assert_eq!(u16::from_le_bytes(ebda_segment), (memory::EBDA_START >> 4) as u16);
    let mut bios_rsdp = [0u8; 20];
    mem.read_at(memory::BIOS_RSDP_ADDR - RAM_BASE, &mut bios_rsdp).unwrap();
    assert_eq!(bios_rsdp, rsdp);

    let mut rsdt_header = [0u8; 40];
    mem.read_at(memory::ACPI_RSDT_ADDR - RAM_BASE, &mut rsdt_header)
        .unwrap();
    assert_eq!(&rsdt_header[0..4], b"RSDT");
    assert_eq!(checksum(&rsdt_header), 0);
    assert_eq!(
        u32::from_le_bytes(rsdt_header[36..40].try_into().unwrap()),
        memory::ACPI_MADT_ADDR as u32
    );

    let mut madt_header = [0u8; 36];
    mem.read_at(memory::ACPI_MADT_ADDR - RAM_BASE, &mut madt_header)
        .unwrap();
    let madt_len = u32::from_le_bytes(madt_header[4..8].try_into().unwrap()) as usize;
    let mut madt = vec![0u8; madt_len];
    mem.read_at(memory::ACPI_MADT_ADDR - RAM_BASE, &mut madt).unwrap();
    assert_eq!(&madt[0..4], b"APIC");
    assert_eq!(checksum(&madt), 0);
    assert_eq!(
        u32::from_le_bytes(madt[36..40].try_into().unwrap()),
        memory::LOCAL_APIC_ADDR
    );

    let lapic_entries = madt[44..]
        .chunks_exact(8)
        .take_while(|entry| entry[0] == 0)
        .collect::<Vec<_>>();
    assert_eq!(lapic_entries.len(), 4);
    for (idx, entry) in lapic_entries.iter().enumerate() {
        assert_eq!(entry[1], 8);
        assert_eq!(entry[2], idx as u8);
        assert_eq!(entry[3], idx as u8);
        assert_eq!(u32::from_le_bytes(entry[4..8].try_into().unwrap()), 1);
    }
}

#[test]
fn acpi_tables_reject_zero_vcpus() {
    let mem = GuestMemory::new(1024 * 1024).unwrap();
    assert!(write_acpi_tables(&mem, 0).is_err());
}

#[test]
fn cpuid_topology_uses_guest_vcpu_ids() {
    let mut entries = vec![
        sys::KvmCpuidEntry2 {
            function: 0x1,
            ebx: 0x0900_0000,
            ..Default::default()
        },
        sys::KvmCpuidEntry2 {
            function: 0xB,
            index: 0,
            ebx: 2,
            edx: 9,
            ..Default::default()
        },
        sys::KvmCpuidEntry2 {
            function: 0xB,
            index: 1,
            ebx: 8,
            edx: 9,
            ..Default::default()
        },
    ];

    configure_cpuid_topology(&mut entries, 2, 4);

    assert_eq!((entries[0].ebx >> 24) & 0xFF, 2);
    assert_eq!((entries[0].ebx >> 16) & 0xFF, 4);
    assert_eq!(entries[1].edx, 2);
    assert_eq!(entries[1].ebx, 2);
    assert_eq!(entries[2].edx, 2);
    assert_eq!(entries[2].ebx, 4);
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b))
}

fn create_fake_bzimage() -> Vec<u8> {
    let mut kernel = vec![0u8; 4096]; // Minimal size

    // Set setup_sects = 4
    kernel[SETUP_HEADER_OFFSET] = 4;

    // Set magic "HdrS"
    kernel[0x202..0x206].copy_from_slice(&HDRS_MAGIC.to_le_bytes());

    // Set boot protocol version (0x0206)
    kernel[0x206..0x208].copy_from_slice(&0x0206u16.to_le_bytes());

    kernel
}

#[test]
fn load_kernel_rejects_bad_magic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vmlinuz");

    let mut kernel = create_fake_bzimage();
    kernel[0x202..0x206].copy_from_slice(&0xDEADBEEFu32.to_le_bytes()); // Break magic
    std::fs::write(&path, &kernel).unwrap();

    let mem = GuestMemory::new(16 * 1024 * 1024).unwrap();
    let result = load_kernel(&mem, &path);
    assert!(result.is_err(), "Should reject kernel without HdrS magic");
    assert!(result.unwrap_err().to_string().contains("bad magic"));
}

#[test]
fn load_kernel_rejects_old_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vmlinuz");

    let mut kernel = create_fake_bzimage();
    kernel[0x206..0x208].copy_from_slice(&0x0205u16.to_le_bytes()); // Protocol 2.05 (too old)
    std::fs::write(&path, &kernel).unwrap();

    let mem = GuestMemory::new(16 * 1024 * 1024).unwrap();
    let result = load_kernel(&mem, &path);
    assert!(result.is_err(), "Should reject boot protocol < 2.06");
    assert!(result.unwrap_err().to_string().contains("boot protocol"));
}

#[test]
fn load_kernel_returns_64_bit_entry_offset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vmlinuz");

    let kernel = create_fake_bzimage();
    std::fs::write(&path, &kernel).unwrap();

    let mem = GuestMemory::new(16 * 1024 * 1024).unwrap();
    let info = load_kernel(&mem, &path).unwrap();

    assert_eq!(info.entry_addr, KERNEL_LOAD_ADDR + 0x200);

    // setup_header should be extracted (0x1F1..0x2B9 = 200 bytes)
    assert_eq!(info.setup_header.len(), 0x2B9 - 0x1F1);
    // First byte is setup_sects (we set it to 4)
    assert_eq!(info.setup_header[0], 4);
}

#[test]
fn build_cmdline_appends_virtio_devices() {
    let cmdline = build_cmdline("console=ttyS0", 2, true);
    assert!(cmdline.starts_with("console=ttyS0"));
    assert!(cmdline.contains("virtio_mmio.device="));
    // Should have 2 device descriptors
    assert_eq!(cmdline.matches("virtio_mmio.device=").count(), 2);
    assert!(!cmdline.contains("no_timer_check"));
}

#[test]
fn build_cmdline_no_devices() {
    let cmdline = build_cmdline("console=ttyS0", 0, true);
    assert_eq!(cmdline, "console=ttyS0");
}

#[test]
fn build_cmdline_no_pit() {
    let cmdline = build_cmdline("console=ttyS0", 1, false);
    assert!(cmdline.contains("no_timer_check"));
    assert!(cmdline.contains("virtio_mmio.device="));
}

#[test]
fn page_tables_cover_4gb_ram() {
    let mem = GuestMemory::new(1024 * 1024).unwrap(); // 1MB is enough for boot structs
    write_page_tables(&mem, 4 * 1024 * 1024 * 1024).unwrap();

    let mut buf = [0u8; 8];
    mem.read_at(PDPT_ADDR - RAM_BASE + 8, &mut buf).unwrap(); // index 1 (1GB-2GB)
    let pdpt_entry = u64::from_le_bytes(buf);
    assert_ne!(
        pdpt_entry & 0x1,
        0,
        "PDPT entry 1 is missing, page tables only cover 1GB"
    );
}

#[test]
fn load_kernel_rejects_arm64_image() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vmlinuz");
    let mut kernel = vec![0u8; 4096];
    // ARM64 Image magic at offset 56
    kernel[56..60].copy_from_slice(&0x644d5241u32.to_le_bytes());
    std::fs::write(&path, &kernel).unwrap();
    let mem = GuestMemory::new(64 * 1024 * 1024).unwrap();
    let err = load_kernel(&mem, &path).unwrap_err();
    assert!(
        err.to_string().contains("not a bzImage"),
        "should reject ARM64 kernel: {err}"
    );
}
