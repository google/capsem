//! x86_64 bzImage boot protocol implementation.
//!
//! Parses bzImage kernel format, constructs boot_params zero page,
//! builds identity-mapped page tables and GDT, and sets initial
//! vCPU register state for 64-bit long mode entry.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::memory::{
    self, GuestMemory, BOOT_PARAMS_ADDR, CMDLINE_ADDR, CMDLINE_MAX_SIZE,
    GDT_ADDR, KERNEL_LOAD_ADDR, PD_ADDR, PDPT_ADDR, PML4_ADDR,
    RAM_BASE, page_align_up,
};
use super::sys;

// ---------------------------------------------------------------------------
// bzImage constants
// ---------------------------------------------------------------------------

/// Magic number in the setup header ("HdrS").
const HDRS_MAGIC: u32 = 0x5372_6448;
/// Offset of the setup header within the boot sector.
const SETUP_HEADER_OFFSET: usize = 0x1F1;
/// Minimum boot protocol version we support (2.06+).
const MIN_BOOT_PROTOCOL: u16 = 0x0206;

/// Kernel load info returned after loading.
pub(super) struct KernelLoadInfo {
    pub entry_addr: u64,
    pub kernel_end: u64,
    /// Raw setup header bytes (offsets 0x1F1..0x2B9 of the bzImage).
    /// Preserved into boot_params so the kernel sees its own header fields.
    pub setup_header: Vec<u8>,
}

/// Initrd load info returned after loading.
pub(super) struct InitrdLoadInfo {
    pub addr: u64,
    pub size: u64,
}

// ---------------------------------------------------------------------------
// Kernel loading
// ---------------------------------------------------------------------------

/// Load a bzImage kernel into guest memory.
pub(super) fn load_kernel(mem: &GuestMemory, kernel_path: &Path) -> Result<KernelLoadInfo> {
    let kernel_data = std::fs::read(kernel_path)
        .with_context(|| format!("reading kernel: {}", kernel_path.display()))?;

    if kernel_data.len() < SETUP_HEADER_OFFSET + 15 {
        bail!("kernel image too small for bzImage header");
    }

    // Check magic
    let magic = u32::from_le_bytes([
        kernel_data[0x202], kernel_data[0x203],
        kernel_data[0x204], kernel_data[0x205],
    ]);
    if magic != HDRS_MAGIC {
        bail!("not a bzImage: bad magic 0x{magic:08x}, expected 0x{HDRS_MAGIC:08x}");
    }

    // Check protocol version
    let version = u16::from_le_bytes([kernel_data[0x206], kernel_data[0x207]]);
    if version < MIN_BOOT_PROTOCOL {
        bail!("boot protocol 0x{version:04x} too old, need >= 0x{MIN_BOOT_PROTOCOL:04x}");
    }

    // Parse setup_sects (number of 512-byte setup sectors, 0 means 4)
    let setup_sects = if kernel_data[SETUP_HEADER_OFFSET] == 0 {
        4u32
    } else {
        kernel_data[SETUP_HEADER_OFFSET] as u32
    };

    // Protected-mode kernel starts after boot sector + setup sectors
    let kernel_offset = (setup_sects as usize + 1) * 512;
    if kernel_offset >= kernel_data.len() {
        bail!("setup_sects ({setup_sects}) exceeds kernel size");
    }

    let protected_mode = &kernel_data[kernel_offset..];
    let kernel_size = protected_mode.len() as u64;

    // Load protected-mode kernel at KERNEL_LOAD_ADDR
    let load_offset = KERNEL_LOAD_ADDR - RAM_BASE;
    if load_offset + kernel_size > mem.size() {
        bail!("kernel ({kernel_size} bytes) exceeds guest memory");
    }
    mem.write_at(load_offset, protected_mode)?;

    // Extract setup header bytes (0x1F1..0x2B9) for boot_params preservation.
    // The kernel reads fields from its own setup header at boot; we must copy
    // them into boot_params so fields like vid_mode, heap_end_ptr, etc. survive.
    const SETUP_HEADER_START: usize = 0x1F1;
    const SETUP_HEADER_END: usize = 0x2B9;
    let setup_header = if kernel_data.len() >= SETUP_HEADER_END {
        kernel_data[SETUP_HEADER_START..SETUP_HEADER_END].to_vec()
    } else if kernel_data.len() > SETUP_HEADER_START {
        kernel_data[SETUP_HEADER_START..].to_vec()
    } else {
        Vec::new()
    };

    Ok(KernelLoadInfo {
        // 64-bit entry point: startup_64 is at offset 0x200 from the
        // protected-mode kernel start (Linux boot protocol >= 2.06).
        entry_addr: KERNEL_LOAD_ADDR + 0x200,
        kernel_end: KERNEL_LOAD_ADDR + kernel_size,
        setup_header,
    })
}

/// Load initrd into guest memory at the end of RAM (page-aligned).
pub(super) fn load_initrd(
    mem: &GuestMemory,
    initrd_path: &Path,
    kernel_end: u64,
) -> Result<InitrdLoadInfo> {
    let initrd_data = std::fs::read(initrd_path)
        .with_context(|| format!("reading initrd: {}", initrd_path.display()))?;

    let initrd_size = initrd_data.len() as u64;
    let ram_end = RAM_BASE + mem.size();

    // Place initrd at end of RAM, page-aligned
    let initrd_addr = memory::page_align_down(ram_end - initrd_size);
    if initrd_addr < kernel_end {
        bail!("initrd overlaps kernel (initrd@{initrd_addr:#x}, kernel_end@{kernel_end:#x})");
    }

    let offset = initrd_addr - RAM_BASE;
    mem.write_at(offset, &initrd_data)?;

    Ok(InitrdLoadInfo {
        addr: initrd_addr,
        size: initrd_size,
    })
}

// ---------------------------------------------------------------------------
// boot_params (zero page) construction
// ---------------------------------------------------------------------------

/// Write the boot_params zero page and kernel cmdline into guest memory.
///
/// `setup_header` contains the raw bytes from the bzImage at offsets
/// 0x1F1..0x2B9. These are copied first, then our bootloader fields
/// (type_of_loader, loadflags, cmd_line_ptr, etc.) are overlaid on top.
pub(super) fn write_boot_params(
    mem: &GuestMemory,
    cmdline: &str,
    initrd: Option<&InitrdLoadInfo>,
    e820_entries: &[memory::E820Entry],
    setup_header: &[u8],
) -> Result<()> {
    let cmdline_bytes = cmdline.as_bytes();
    if cmdline_bytes.len() as u64 >= CMDLINE_MAX_SIZE {
        bail!("kernel cmdline too long ({} bytes)", cmdline_bytes.len());
    }

    // Write cmdline (null-terminated)
    let mut cmdline_buf = Vec::with_capacity(cmdline_bytes.len() + 1);
    cmdline_buf.extend_from_slice(cmdline_bytes);
    cmdline_buf.push(0);
    mem.write_at(CMDLINE_ADDR - RAM_BASE, &cmdline_buf)?;

    // Build boot_params (zero page) -- 4096 bytes, mostly zeros
    let mut params = vec![0u8; 4096];

    // Preserve original setup_header from the bzImage so the kernel sees
    // its own vid_mode, heap_end_ptr, and other self-describing fields.
    if !setup_header.is_empty() {
        let dest = 0x1F1;
        let len = setup_header.len().min(4096 - dest);
        params[dest..dest + len].copy_from_slice(&setup_header[..len]);
    }

    // Overlay our bootloader fields on top of the preserved header.
    // type_of_loader at offset 0x210
    params[0x210] = 0xFF;

    // loadflags at offset 0x211: set LOADED_HIGH (bit 0) + CAN_USE_HEAP (bit 7)
    params[0x211] = 0x81;

    // cmd_line_ptr at offset 0x228 (u32)
    let cmdline_ptr = CMDLINE_ADDR as u32;
    params[0x228..0x22C].copy_from_slice(&cmdline_ptr.to_le_bytes());

    // cmdline_size at offset 0x238 (u32)
    let cmdline_size = cmdline_bytes.len() as u32;
    params[0x238..0x23C].copy_from_slice(&cmdline_size.to_le_bytes());

    // initrd
    if let Some(initrd) = initrd {
        // ramdisk_image at offset 0x218 (u32)
        let ramdisk_image = initrd.addr as u32;
        params[0x218..0x21C].copy_from_slice(&ramdisk_image.to_le_bytes());
        // ramdisk_size at offset 0x21C (u32)
        let ramdisk_size = initrd.size as u32;
        params[0x21C..0x220].copy_from_slice(&ramdisk_size.to_le_bytes());
    }

    // E820 map: e820_entries at offset 0x2D0, each entry is 20 bytes
    // e820_nr at offset 0x1E8 (u8)
    let nr_entries = e820_entries.len().min(128) as u8;
    params[0x1E8] = nr_entries;
    for (i, entry) in e820_entries.iter().enumerate().take(128) {
        let base = 0x2D0 + i * 20;
        params[base..base + 8].copy_from_slice(&entry.addr.to_le_bytes());
        params[base + 8..base + 16].copy_from_slice(&entry.size.to_le_bytes());
        params[base + 16..base + 20].copy_from_slice(&entry.type_.to_le_bytes());
    }

    mem.write_at(BOOT_PARAMS_ADDR - RAM_BASE, &params)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// GDT and page tables
// ---------------------------------------------------------------------------

/// Write a minimal GDT (null + code64 + data) into guest memory.
pub(super) fn write_gdt(mem: &GuestMemory) -> Result<()> {
    let gdt: [u64; 3] = [
        0x0000_0000_0000_0000, // null descriptor
        0x00209A00_00000000,   // 64-bit code: L=1, execute/read, present
        0x00009200_00000000,   // data: read/write, present
    ];
    let bytes: Vec<u8> = gdt.iter().flat_map(|v| v.to_le_bytes()).collect();
    mem.write_at(GDT_ADDR - RAM_BASE, &bytes)?;
    Ok(())
}

/// Write identity-mapped page tables (PML4 -> PDPT -> PD) covering all guest RAM.
/// Uses 2 MiB huge pages.
pub(super) fn write_page_tables(mem: &GuestMemory, ram_size: u64) -> Result<()> {
    // PML4[0] -> PDPT
    let pml4_entry: u64 = PDPT_ADDR | 0x3; // present + writable
    mem.write_at(PML4_ADDR - RAM_BASE, &pml4_entry.to_le_bytes())?;

    // 1 PDPT entry = 1 GB (maps to 1 PD page)
    // 1 PD page = 512 PD entries = 512 * 2MB = 1GB
    let gb_count = (ram_size + 0x3FFF_FFFF) / 0x4000_0000;

    let mut pdpt = vec![0u8; 4096];
    for i in 0..gb_count {
        let pd_addr = PD_ADDR + i * 4096;
        let entry: u64 = pd_addr | 0x3;
        let offset = (i as usize) * 8;
        pdpt[offset..offset + 8].copy_from_slice(&entry.to_le_bytes());
    }
    mem.write_at(PDPT_ADDR - RAM_BASE, &pdpt)?;

    let mut pd = vec![0u8; (gb_count * 4096) as usize];
    let total_pages = (ram_size + 0x1F_FFFF) / 0x20_0000;
    
    for i in 0..total_pages {
        let entry: u64 = (i << 21) | 0x83; // present + writable + huge page (PS bit)
        let offset = (i as usize) * 8;
        pd[offset..offset + 8].copy_from_slice(&entry.to_le_bytes());
    }
    mem.write_at(PD_ADDR - RAM_BASE, &pd)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// vCPU register setup
// ---------------------------------------------------------------------------

/// Configure vCPU registers for 64-bit long mode boot.
pub(super) fn setup_boot_regs(
    vcpu: &sys::VcpuFd,
    entry_addr: u64,
    boot_params_addr: u64,
) -> Result<()> {
    // Code segment: selector 0x08 (GDT entry 1)
    let code_seg = sys::KvmSegment {
        base: 0,
        limit: 0xFFFF_FFFF,
        selector: 0x08,
        type_: 11, // execute/read, accessed
        present: 1,
        dpl: 0,
        db: 0,
        s: 1,
        l: 1, // long mode
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    };

    // Data segment: selector 0x10 (GDT entry 2)
    let data_seg = sys::KvmSegment {
        base: 0,
        limit: 0xFFFF_FFFF,
        selector: 0x10,
        type_: 3, // read/write, accessed
        present: 1,
        dpl: 0,
        db: 1,
        s: 1,
        l: 0,
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    };

    let mut sregs = sys::KvmSregs::default();
    sregs.cs = code_seg;
    sregs.ds = data_seg;
    sregs.es = data_seg;
    sregs.fs = data_seg;
    sregs.gs = data_seg;
    sregs.ss = data_seg;

    sregs.gdt = sys::KvmDtable {
        base: GDT_ADDR,
        limit: 23, // 3 entries * 8 bytes - 1
        padding: [0; 3],
    };

    // Control registers for 64-bit paging
    sregs.cr0 = 0x8000_0001; // PG (paging) + PE (protected mode)
    sregs.cr3 = PML4_ADDR;   // page table base
    sregs.cr4 = 0x20;        // PAE (physical address extension)

    // EFER: LME (long mode enable) + LMA (long mode active)
    sregs.efer = 0x500;

    vcpu.set_sregs(&sregs)?;

    // General-purpose registers
    let regs = sys::KvmRegs {
        rip: entry_addr,
        rsi: boot_params_addr,
        rflags: 0x2, // reserved bit must be set
        ..Default::default()
    };
    vcpu.set_regs(&regs)?;

    Ok(())
}

/// Set up CPUID for a vCPU (passthrough host CPUID entries).
pub(super) fn setup_cpuid(vm: &sys::VmFd, vcpu: &sys::VcpuFd) -> Result<()> {
    let entries = vm.get_supported_cpuid()?;
    vcpu.set_cpuid2(&entries)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// High-level boot orchestration
// ---------------------------------------------------------------------------

/// Build kernel command line with virtio MMIO device descriptors appended.
pub(super) fn build_cmdline(
    base_cmdline: &str,
    virtio_device_count: u32,
) -> String {
    let mut cmdline = base_cmdline.to_string();
    for slot in 0..virtio_device_count {
        let addr = memory::virtio_mmio_addr(slot);
        let irq = memory::virtio_mmio_irq(slot);
        cmdline.push_str(&format!(
            " virtio_mmio.device=0x{:x}@0x{:x}:{}",
            memory::VIRTIO_MMIO_SIZE, addr, irq
        ));
    }
    cmdline
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gdt_entries_correct_size() {
        // 3 entries * 8 bytes = 24 bytes
        let mem = GuestMemory::new(4096 * 16).unwrap();
        write_gdt(&mem).unwrap();
        let mut buf = [0u8; 24];
        mem.read_at(GDT_ADDR - RAM_BASE, &mut buf).unwrap();
        // Null entry should be zero
        assert_eq!(&buf[..8], &[0u8; 8]);
        // Code64 entry should be non-zero
        assert_ne!(&buf[8..16], &[0u8; 8]);
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
        let initrd = InitrdLoadInfo { addr: 0x80_0000, size: 1024 * 1024 };
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
        fake_header[fake_header.len() - 1] = 0xBB;
        
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
    fn load_kernel_returns_correct_entry_offset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vmlinuz");
        
        let kernel = create_fake_bzimage();
        std::fs::write(&path, &kernel).unwrap();

        let mem = GuestMemory::new(16 * 1024 * 1024).unwrap();
        let info = load_kernel(&mem, &path).unwrap();

        // 64-bit entry point MUST be at exactly KERNEL_LOAD_ADDR + 0x200
        assert_eq!(info.entry_addr, KERNEL_LOAD_ADDR + 0x200);

        // setup_header should be extracted (0x1F1..0x2B9 = 200 bytes)
        assert_eq!(info.setup_header.len(), 0x2B9 - 0x1F1);
        // First byte is setup_sects (we set it to 4)
        assert_eq!(info.setup_header[0], 4);
    }

    #[test]
    fn build_cmdline_appends_virtio_devices() {
        let cmdline = build_cmdline("console=ttyS0", 2);
        assert!(cmdline.starts_with("console=ttyS0"));
        assert!(cmdline.contains("virtio_mmio.device="));
        // Should have 2 device descriptors
        assert_eq!(cmdline.matches("virtio_mmio.device=").count(), 2);
    }

    #[test]
    fn build_cmdline_no_devices() {
        let cmdline = build_cmdline("console=ttyS0", 0);
        assert_eq!(cmdline, "console=ttyS0");
    }

    #[test]
    fn page_tables_cover_4gb_ram() {
        let mem = GuestMemory::new(1024 * 1024).unwrap(); // 1MB is enough for boot structs
        write_page_tables(&mem, 4 * 1024 * 1024 * 1024).unwrap();

        let mut buf = [0u8; 8];
        mem.read_at(PDPT_ADDR - RAM_BASE + 8, &mut buf).unwrap(); // index 1 (1GB-2GB)
        let pdpt_entry = u64::from_le_bytes(buf);
        assert_ne!(pdpt_entry & 0x1, 0, "PDPT entry 1 is missing, page tables only cover 1GB");
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
        assert!(err.to_string().contains("not a bzImage"), "should reject ARM64 kernel: {err}");
    }
}
