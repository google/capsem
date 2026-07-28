use super::*;

// -----------------------------------------------------------------------
// ARM64 header parsing
// -----------------------------------------------------------------------

#[test]
fn parse_header_too_short() {
    let data = vec![0u8; 32]; // less than 64 bytes
    assert_eq!(parse_arm64_header(&data), memory::KERNEL_TEXT_OFFSET);
}

#[test]
fn parse_header_empty() {
    assert_eq!(parse_arm64_header(&[]), memory::KERNEL_TEXT_OFFSET);
}

#[test]
fn parse_header_no_magic() {
    let data = vec![0u8; 64];
    assert_eq!(parse_arm64_header(&data), memory::KERNEL_TEXT_OFFSET);
}

#[test]
fn parse_header_wrong_magic() {
    let mut data = vec![0u8; 64];
    // Put wrong magic at offset 56
    data[56..60].copy_from_slice(&0xDEADBEEFu32.to_le_bytes());
    assert_eq!(parse_arm64_header(&data), memory::KERNEL_TEXT_OFFSET);
}

#[test]
fn parse_header_valid_magic_zero_offset() {
    let mut data = vec![0u8; 64];
    // Set ARM64 magic at offset 56
    data[56..60].copy_from_slice(&ARM64_IMAGE_MAGIC.to_le_bytes());
    // text_offset = 0 at bytes 8-15 -> should use default
    assert_eq!(parse_arm64_header(&data), memory::KERNEL_TEXT_OFFSET);
}

#[test]
fn parse_header_valid_magic_standard_offset() {
    let mut data = vec![0u8; 64];
    data[56..60].copy_from_slice(&ARM64_IMAGE_MAGIC.to_le_bytes());
    // text_offset = 0x80000 at bytes 8-15
    data[8..16].copy_from_slice(&0x80000u64.to_le_bytes());
    assert_eq!(parse_arm64_header(&data), 0x80000);
}

#[test]
fn parse_header_valid_magic_custom_offset() {
    let mut data = vec![0u8; 64];
    data[56..60].copy_from_slice(&ARM64_IMAGE_MAGIC.to_le_bytes());
    // text_offset = 0x200000 (2MB, some kernels use this)
    data[8..16].copy_from_slice(&0x200000u64.to_le_bytes());
    assert_eq!(parse_arm64_header(&data), 0x200000);
}

#[test]
fn parse_header_absurdly_large_offset() {
    let mut data = vec![0u8; 64];
    data[56..60].copy_from_slice(&ARM64_IMAGE_MAGIC.to_le_bytes());
    // text_offset = 1GB -- too large, should fall back
    data[8..16].copy_from_slice(&(1024u64 * 1024 * 1024).to_le_bytes());
    assert_eq!(parse_arm64_header(&data), memory::KERNEL_TEXT_OFFSET);
}

// -----------------------------------------------------------------------
// Kernel loading
// -----------------------------------------------------------------------

#[test]
fn load_kernel_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let kernel_path = dir.path().join("vmlinuz");

    // Create a fake kernel with ARM64 header
    let mut kernel = vec![0u8; 1024];
    kernel[56..60].copy_from_slice(&ARM64_IMAGE_MAGIC.to_le_bytes());
    kernel[8..16].copy_from_slice(&0x80000u64.to_le_bytes());
    std::fs::write(&kernel_path, &kernel).unwrap();

    let mem = GuestMemory::new(64 * 1024 * 1024).unwrap(); // 64MB
    let info = load_kernel(&mem, &kernel_path).unwrap();

    assert_eq!(info.entry_addr, memory::RAM_BASE + 0x80000);
    assert_eq!(info.kernel_end, memory::RAM_BASE + 0x80000 + 1024);
}

#[test]
fn load_kernel_without_magic_uses_default_offset() {
    let dir = tempfile::tempdir().unwrap();
    let kernel_path = dir.path().join("vmlinuz");

    let kernel = vec![0xCC; 512]; // no ARM64 magic
    std::fs::write(&kernel_path, &kernel).unwrap();

    let mem = GuestMemory::new(64 * 1024 * 1024).unwrap();
    let info = load_kernel(&mem, &kernel_path).unwrap();

    assert_eq!(
        info.entry_addr,
        memory::RAM_BASE + memory::KERNEL_TEXT_OFFSET
    );
}

#[test]
fn load_kernel_empty_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let kernel_path = dir.path().join("vmlinuz");
    std::fs::write(&kernel_path, b"").unwrap();

    let mem = GuestMemory::new(4096).unwrap();
    assert!(load_kernel(&mem, &kernel_path).is_err());
}

#[test]
fn load_kernel_nonexistent_file_fails() {
    let mem = GuestMemory::new(4096).unwrap();
    assert!(load_kernel(&mem, Path::new("/nonexistent/vmlinuz")).is_err());
}

#[test]
fn load_kernel_too_large_fails() {
    let dir = tempfile::tempdir().unwrap();
    let kernel_path = dir.path().join("vmlinuz");

    // 1MB kernel but only 512KB of RAM (after text_offset, won't fit)
    let kernel = vec![0u8; 1024 * 1024];
    std::fs::write(&kernel_path, &kernel).unwrap();

    let mem = GuestMemory::new(512 * 1024).unwrap(); // 512KB RAM
    assert!(load_kernel(&mem, &kernel_path).is_err());
}

#[test]
fn load_kernel_rejects_bzimage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vmlinuz");
    let mut kernel = vec![0u8; 4096];
    // bzImage HdrS magic at offset 0x202
    kernel[0x202..0x206].copy_from_slice(&0x5372_6448u32.to_le_bytes());
    std::fs::write(&path, &kernel).unwrap();
    let mem = GuestMemory::new(64 * 1024 * 1024).unwrap();
    let err = load_kernel(&mem, &path).unwrap_err();
    assert!(
        err.to_string().contains("bzImage"),
        "should reject bzImage kernel: {err}"
    );
}

// -----------------------------------------------------------------------
// Initrd loading
// -----------------------------------------------------------------------

#[test]
fn load_initrd_at_end_of_ram() {
    let dir = tempfile::tempdir().unwrap();
    let initrd_path = dir.path().join("initrd.img");
    let initrd_data = vec![0xAA; 8192]; // 8KB initrd
    std::fs::write(&initrd_path, &initrd_data).unwrap();

    let ram_size: u64 = 64 * 1024 * 1024; // 64MB
    let mem = GuestMemory::new(ram_size).unwrap();
    let kernel_end = memory::RAM_BASE + memory::KERNEL_TEXT_OFFSET + 1024;
    let info = load_initrd(&mem, &initrd_path, kernel_end).unwrap();

    // Should be page-aligned
    assert_eq!(info.guest_addr % memory::PAGE_SIZE, 0);
    // Should be near end of RAM
    assert!(info.guest_addr + info.size as u64 <= memory::RAM_BASE + ram_size);
    assert_eq!(info.size, 8192);
}

#[test]
fn load_initrd_empty_fails() {
    let dir = tempfile::tempdir().unwrap();
    let initrd_path = dir.path().join("initrd.img");
    std::fs::write(&initrd_path, b"").unwrap();

    let mem = GuestMemory::new(64 * 1024 * 1024).unwrap();
    let kernel_end = memory::RAM_BASE + memory::KERNEL_TEXT_OFFSET;
    assert!(load_initrd(&mem, &initrd_path, kernel_end).is_err());
}

#[test]
fn load_initrd_nonexistent_fails() {
    let mem = GuestMemory::new(4096).unwrap();
    let kernel_end = memory::RAM_BASE + memory::KERNEL_TEXT_OFFSET;
    assert!(load_initrd(&mem, Path::new("/nonexistent/initrd"), kernel_end).is_err());
}

#[test]
fn load_initrd_overlaps_kernel_fails() {
    let dir = tempfile::tempdir().unwrap();
    let initrd_path = dir.path().join("initrd.img");
    let initrd_data = vec![0xAA; 32 * 1024 * 1024]; // 32MB initrd
    std::fs::write(&initrd_path, &initrd_data).unwrap();

    let ram_size: u64 = 64 * 1024 * 1024; // 64MB RAM
    let mem = GuestMemory::new(ram_size).unwrap();

    // Push kernel_end to 40MB. Initrd needs 32MB, but we only have 64MB total.
    // 64MB - 32MB = 32MB available start. 32MB < 40MB (overlap).
    let kernel_end = memory::RAM_BASE + 40 * 1024 * 1024;
    let result = load_initrd(&mem, &initrd_path, kernel_end);

    assert!(
        result.is_err(),
        "Should reject initrd if it overlaps the kernel"
    );
    assert!(result.unwrap_err().to_string().contains("too large to fit"));
}

// -----------------------------------------------------------------------
// FDT loading
// -----------------------------------------------------------------------

#[test]
fn load_fdt_after_kernel() {
    let ram_size: u64 = 64 * 1024 * 1024;
    let mem = GuestMemory::new(ram_size).unwrap();
    let fdt_blob = vec![0xd0, 0x0d, 0xfe, 0xed, 0, 0, 0, 0]; // fake FDT header

    let kernel_end = memory::RAM_BASE + memory::KERNEL_TEXT_OFFSET + 1024;
    let fdt_addr = load_fdt(&mem, &fdt_blob, kernel_end).unwrap();

    // FDT should be page-aligned
    assert_eq!(fdt_addr % memory::PAGE_SIZE, 0);
    // FDT should be after kernel end
    assert!(fdt_addr >= kernel_end);
}

#[test]
fn load_fdt_too_large_fails() {
    let ram_size: u64 = 4096;
    let mem = GuestMemory::new(ram_size).unwrap();
    let fdt_blob = vec![0u8; 8192]; // larger than RAM

    let kernel_end = memory::RAM_BASE + memory::KERNEL_TEXT_OFFSET;
    assert!(load_fdt(&mem, &fdt_blob, kernel_end).is_err());
}

#[test]
fn load_fdt_exceeds_512mb_distance() {
    // Create 2GB RAM so memory size itself doesn't cause failure
    let ram_size: u64 = 2 * 1024 * 1024 * 1024;
    let mem = GuestMemory::new(ram_size).unwrap();
    let fdt_blob = vec![0xd0, 0x0d, 0xfe, 0xed, 0, 0, 0, 0]; // fake FDT

    // Push kernel_end beyond 512MB limit from kernel_entry (0x80000)
    let kernel_end = memory::RAM_BASE + memory::KERNEL_TEXT_OFFSET + 513 * 1024 * 1024;
    let result = load_fdt(&mem, &fdt_blob, kernel_end);

    assert!(
        result.is_err(),
        "Should reject FDT that is > 512MB away from kernel entry"
    );
    assert!(result.unwrap_err().to_string().contains("more than 512MB"));
}

// -----------------------------------------------------------------------
// Register values
// -----------------------------------------------------------------------

#[test]
fn boot_regs_are_correct_values() {
    // Just verify the register IDs we'd use are the right constants
    assert_eq!(sys::REG_PC, 0x6030_0000_0010_0040);
    assert_eq!(sys::REG_X0, 0x6030_0000_0010_0000);
    assert_eq!(sys::REG_X1, 0x6030_0000_0010_0002);
    assert_eq!(sys::REG_X2, 0x6030_0000_0010_0004);
    assert_eq!(sys::REG_X3, 0x6030_0000_0010_0006);
}
