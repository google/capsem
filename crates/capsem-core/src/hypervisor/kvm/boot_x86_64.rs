//! x86_64 bzImage boot protocol implementation.
//!
//! Parses bzImage kernel format, constructs boot_params zero page,
//! builds identity-mapped page tables and GDT, and sets initial
//! vCPU register state for 64-bit long mode entry.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::memory::{
    self, GuestMemory, BOOT_PARAMS_ADDR, CMDLINE_ADDR, CMDLINE_MAX_SIZE, GDT_ADDR,
    KERNEL_LOAD_ADDR, PDPT_ADDR, PD_ADDR, PML4_ADDR, RAM_BASE,
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
#[derive(Debug)]
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
        kernel_data[0x202],
        kernel_data[0x203],
        kernel_data[0x204],
        kernel_data[0x205],
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
        // 64-bit boot protocol entry: startup_64 is at offset 0x200 from the
        // protected-mode kernel start for bzImage kernels.
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
    // Keep the initrd below the 32-bit boot protocol limit and below the
    // x86 PCI/MMIO hole. Linux can later use RAM above 4 GiB from E820.
    let ram_end = RAM_BASE + mem.size().min(memory::PCI_HOLE_START);

    // Place initrd at end of RAM, page-aligned
    let initrd_addr = memory::page_align_down(ram_end - initrd_size);
    if initrd_addr < kernel_end {
        bail!("initrd overlaps kernel (initrd@{initrd_addr:#x}, kernel_end@{kernel_end:#x})");
    }

    mem.write_gpa(initrd_addr, &initrd_data)?;

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
// ACPI tables
// ---------------------------------------------------------------------------

const ACPI_OEM_ID: &[u8; 6] = b"CAPSEM";
const ACPI_OEM_TABLE_ID: &[u8; 8] = b"CAPSEMKV";
const ACPI_CREATOR_ID: &[u8; 4] = b"CAPS";

/// Write a minimal ACPI v1 RSDP/RSDT/MADT table set for x86 SMP discovery.
///
/// Without MADT, Linux boots on CPU0 only even when KVM has additional vCPUs.
/// The application processors remain parked in KVM until Linux reads MADT,
/// discovers their LAPIC IDs, and starts them through INIT/SIPI.
pub(super) fn write_acpi_tables(mem: &GuestMemory, cpu_count: u32) -> Result<()> {
    if cpu_count == 0 || cpu_count > u8::MAX as u32 {
        bail!("ACPI MADT supports 1..=255 vCPUs, got {cpu_count}");
    }

    let madt = build_madt(cpu_count)?;
    let rsdt = build_rsdt(memory::ACPI_MADT_ADDR as u32);
    let rsdp = build_rsdp(memory::ACPI_RSDT_ADDR as u32);
    let ebda_segment = (memory::EBDA_START >> 4) as u16;

    mem.write_gpa(memory::BDA_EBDA_SEGMENT_ADDR, &ebda_segment.to_le_bytes())?;
    mem.write_gpa(memory::ACPI_RSDP_ADDR, &rsdp)?;
    mem.write_gpa(memory::BIOS_RSDP_ADDR, &rsdp)?;
    mem.write_gpa(memory::ACPI_RSDT_ADDR, &rsdt)?;
    mem.write_gpa(memory::ACPI_MADT_ADDR, &madt)?;
    Ok(())
}

fn build_rsdp(rsdt_addr: u32) -> [u8; 20] {
    let mut rsdp = [0u8; 20];
    rsdp[0..8].copy_from_slice(b"RSD PTR ");
    rsdp[9..15].copy_from_slice(ACPI_OEM_ID);
    rsdp[15] = 0; // ACPI 1.0
    rsdp[16..20].copy_from_slice(&rsdt_addr.to_le_bytes());
    fill_checksum(&mut rsdp, 8);
    rsdp
}

fn build_rsdt(madt_addr: u32) -> Vec<u8> {
    let mut rsdt = acpi_table_header(b"RSDT", 36 + 4, 1);
    rsdt.extend_from_slice(&madt_addr.to_le_bytes());
    fill_checksum(&mut rsdt, 9);
    rsdt
}

fn build_madt(cpu_count: u32) -> Result<Vec<u8>> {
    let ioapic_id = cpu_count as u8;
    let entry_bytes = cpu_count as usize * 8 + 12 + 6;
    let mut madt = acpi_table_header(b"APIC", 36 + 8 + entry_bytes, 1);
    madt.extend_from_slice(&memory::LOCAL_APIC_ADDR.to_le_bytes());
    madt.extend_from_slice(&1u32.to_le_bytes()); // PC-AT compatible dual-PIC flag

    for cpu_id in 0..cpu_count {
        madt.push(0); // Processor Local APIC
        madt.push(8);
        madt.push(cpu_id as u8); // ACPI processor UID
        madt.push(cpu_id as u8); // APIC ID
        madt.extend_from_slice(&1u32.to_le_bytes()); // enabled
    }

    madt.push(1); // IOAPIC
    madt.push(12);
    madt.push(ioapic_id);
    madt.push(0);
    madt.extend_from_slice(&memory::IO_APIC_ADDR.to_le_bytes());
    madt.extend_from_slice(&0u32.to_le_bytes()); // GSI base

    madt.push(4); // Local APIC NMI
    madt.push(6);
    madt.push(0xFF); // all processors
    madt.extend_from_slice(&0u16.to_le_bytes()); // polarity/trigger conforming
    madt.push(1); // LINT1

    fill_checksum(&mut madt, 9);
    Ok(madt)
}

fn acpi_table_header(signature: &[u8; 4], length: usize, revision: u8) -> Vec<u8> {
    let mut table = Vec::with_capacity(length);
    table.extend_from_slice(signature);
    table.extend_from_slice(&(length as u32).to_le_bytes());
    table.push(revision);
    table.push(0); // checksum, filled after body is appended
    table.extend_from_slice(ACPI_OEM_ID);
    table.extend_from_slice(ACPI_OEM_TABLE_ID);
    table.extend_from_slice(&1u32.to_le_bytes());
    table.extend_from_slice(ACPI_CREATOR_ID);
    table.extend_from_slice(&1u32.to_le_bytes());
    table
}

fn fill_checksum(bytes: &mut [u8], checksum_offset: usize) {
    bytes[checksum_offset] = 0;
    let sum = bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
    bytes[checksum_offset] = 0u8.wrapping_sub(sum);
}

// ---------------------------------------------------------------------------
// GDT and page tables
// ---------------------------------------------------------------------------

/// Write a minimal long-mode GDT using Linux boot selectors.
pub(super) fn write_gdt(mem: &GuestMemory) -> Result<()> {
    let gdt: [u64; 4] = [
        0x0000_0000_0000_0000, // null descriptor
        0x0000_0000_0000_0000, // unused: Linux boot protocol uses CS=0x10
        0x0020_9A00_0000_0000, // 64-bit flat code: execute/read, present
        0x0000_9200_0000_0000, // data: read/write, present
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
    let gb_count = ram_size.div_ceil(0x4000_0000);

    let mut pdpt = vec![0u8; 4096];
    for i in 0..gb_count {
        let pd_addr = PD_ADDR + i * 4096;
        let entry: u64 = pd_addr | 0x3;
        let offset = (i as usize) * 8;
        pdpt[offset..offset + 8].copy_from_slice(&entry.to_le_bytes());
    }
    mem.write_at(PDPT_ADDR - RAM_BASE, &pdpt)?;

    let mut pd = vec![0u8; (gb_count * 4096) as usize];
    let total_pages = ram_size.div_ceil(0x20_0000);

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

/// Configure vCPU registers for the Linux bzImage 64-bit boot protocol.
pub(super) fn setup_boot_regs(
    vcpu: &sys::VcpuFd,
    entry_addr: u64,
    boot_params_addr: u64,
) -> Result<()> {
    // Linux x86 boot protocol uses __BOOT_CS=0x10 and __BOOT_DS=0x18.
    let code_seg = sys::KvmSegment {
        base: 0,
        limit: 0xFFFF_FFFF,
        selector: 0x10,
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

    // Data segment: selector 0x18 (GDT entry 3)
    let data_seg = sys::KvmSegment {
        base: 0,
        limit: 0xFFFF_FFFF,
        selector: 0x18,
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

    let mut sregs = vcpu.get_sregs()?;
    sregs.cs = code_seg;
    sregs.ds = data_seg;
    sregs.es = data_seg;
    sregs.fs = data_seg;
    sregs.gs = data_seg;
    sregs.ss = data_seg;

    sregs.gdt = sys::KvmDtable {
        base: GDT_ADDR,
        limit: 31, // 4 entries * 8 bytes - 1
        padding: [0; 3],
    };

    // 64-bit boot protocol expects long mode with identity paging enabled.
    sregs.cr0 = 0x8000_0001; // PG (paging) + PE (protected mode)
    sregs.cr3 = PML4_ADDR; // page table base
    sregs.cr4 = 0x20; // PAE (physical address extension)
    sregs.efer = 0x500; // LME + LMA

    vcpu.set_sregs(&sregs)?;

    // General-purpose registers
    let regs = sys::KvmRegs {
        rip: entry_addr,
        rsi: boot_params_addr,
        rflags: 0x2, // reserved bit must be set
        ..Default::default()
    };
    vcpu.set_regs(&regs)?;
    vcpu.set_mp_state(sys::KvmMpState {
        mp_state: sys::KVM_MP_STATE_RUNNABLE,
    })?;

    Ok(())
}

/// Park an application processor until the guest sends INIT/SIPI via LAPIC.
pub(super) fn setup_application_processor(vcpu: &sys::VcpuFd) -> Result<()> {
    vcpu.set_mp_state(sys::KvmMpState {
        mp_state: sys::KVM_MP_STATE_UNINITIALIZED,
    })
}

/// Set up CPUID for a vCPU.
pub(super) fn setup_cpuid(
    kvm: &sys::KvmFd,
    vcpu: &sys::VcpuFd,
    vcpu_id: u32,
    cpu_count: u32,
) -> Result<()> {
    let mut entries = kvm.get_supported_cpuid()?;
    configure_cpuid_topology(&mut entries, vcpu_id, cpu_count);
    vcpu.set_cpuid2(&entries)?;
    Ok(())
}

fn configure_cpuid_topology(entries: &mut [sys::KvmCpuidEntry2], vcpu_id: u32, cpu_count: u32) {
    let logical_processors = cpu_count.clamp(1, u8::MAX as u32);
    let apic_id = vcpu_id.min(u8::MAX as u32);

    for entry in entries {
        match entry.function {
            0x1 => {
                entry.ebx &= !0x00FF_0000;
                entry.ebx |= logical_processors << 16;
                entry.ebx &= !0xFF00_0000;
                entry.ebx |= apic_id << 24;
            }
            0xB | 0x1F => {
                entry.edx = vcpu_id;
                if entry.index > 0 && entry.ebx != 0 {
                    entry.ebx = cpu_count;
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// High-level boot orchestration
// ---------------------------------------------------------------------------

/// Build kernel command line with virtio MMIO device descriptors appended.
pub(super) fn build_cmdline(base_cmdline: &str, virtio_device_count: u32, has_pit: bool) -> String {
    let mut cmdline = base_cmdline.to_string();
    if !has_pit {
        cmdline.push_str(" no_timer_check");
    }
    for slot in 0..virtio_device_count {
        let addr = memory::virtio_mmio_addr(slot);
        let irq = memory::virtio_mmio_irq(slot);
        cmdline.push_str(&format!(
            " virtio_mmio.device=0x{:x}@0x{:x}:{}",
            memory::VIRTIO_MMIO_SIZE,
            addr,
            irq
        ));
    }
    cmdline
}

#[cfg(test)]
mod tests {
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
        mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x228, &mut ptr_buf)
            .unwrap();
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
        mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x218, &mut buf)
            .unwrap();
        assert_eq!(u32::from_le_bytes(buf), 0x80_0000);
        // Check ramdisk_size
        mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x21C, &mut buf)
            .unwrap();
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
        mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x1f1, &mut buf)
            .unwrap();
        assert_eq!(buf[0], 0xAA, "First byte of setup_header not preserved");

        mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x2b8, &mut buf)
            .unwrap();
        assert_eq!(buf[0], 0xBB, "Last byte of setup_header not preserved");
    }

    #[test]
    fn write_boot_params_sets_loader_and_flags() {
        let mem = GuestMemory::new(4096 * 256).unwrap();
        let e820 = memory::build_e820_map(256 * 4096);
        write_boot_params(&mem, "test", None, &e820, &[]).unwrap();

        let mut buf = [0u8; 1];
        mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x210, &mut buf)
            .unwrap();
        assert_eq!(buf[0], 0xFF, "type_of_loader must be 0xFF");

        mem.read_at(BOOT_PARAMS_ADDR - RAM_BASE + 0x211, &mut buf)
            .unwrap();
        assert_eq!(
            buf[0], 0x81,
            "loadflags must be 0x81 (LOADED_HIGH | CAN_USE_HEAP)"
        );
    }

    #[test]
    fn acpi_tables_advertise_all_vcpus_in_madt() {
        let mem = GuestMemory::new(1024 * 1024).unwrap();
        write_acpi_tables(&mem, 4).unwrap();

        let mut rsdp = [0u8; 20];
        mem.read_at(memory::ACPI_RSDP_ADDR - RAM_BASE, &mut rsdp)
            .unwrap();
        assert_eq!(&rsdp[0..8], b"RSD PTR ");
        assert_eq!(checksum(&rsdp), 0);
        assert_eq!(
            u32::from_le_bytes(rsdp[16..20].try_into().unwrap()),
            memory::ACPI_RSDT_ADDR as u32
        );

        let mut ebda_segment = [0u8; 2];
        mem.read_at(memory::BDA_EBDA_SEGMENT_ADDR - RAM_BASE, &mut ebda_segment)
            .unwrap();
        assert_eq!(
            u16::from_le_bytes(ebda_segment),
            (memory::EBDA_START >> 4) as u16
        );
        let mut bios_rsdp = [0u8; 20];
        mem.read_at(memory::BIOS_RSDP_ADDR - RAM_BASE, &mut bios_rsdp)
            .unwrap();
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
        mem.read_at(memory::ACPI_MADT_ADDR - RAM_BASE, &mut madt)
            .unwrap();
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
}
