//! Guest physical memory layout and management for KVM.
//!
//! Defines the guest physical address map and provides a safe wrapper
//! around the mmap'd guest memory region.

use anyhow::{bail, Result};

// ---------------------------------------------------------------------------
// Shared constants
// ---------------------------------------------------------------------------

/// Page size for alignment.
pub(super) const PAGE_SIZE: u64 = 4096;

/// Size of each virtio MMIO device slot (virtio spec).
pub(super) const VIRTIO_MMIO_SIZE: u64 = 0x200;

/// Maximum number of virtio MMIO device slots.
pub(super) const VIRTIO_MMIO_MAX_DEVICES: u32 = 32;

// ---------------------------------------------------------------------------
// aarch64 guest physical address map
// ---------------------------------------------------------------------------

/// GIC distributor base address (64KB region).
#[cfg(target_arch = "aarch64")]
pub(super) const GIC_DIST_BASE: u64 = 0x0800_0000;
/// GIC distributor region size.
#[cfg(target_arch = "aarch64")]
pub(super) const GIC_DIST_SIZE: u64 = 0x0001_0000; // 64KB

/// GIC redistributor base address (128KB per vCPU).
#[cfg(target_arch = "aarch64")]
pub(super) const GIC_REDIST_BASE: u64 = 0x080A_0000;
/// GIC redistributor size per vCPU.
#[cfg(target_arch = "aarch64")]
pub(super) const GIC_REDIST_PER_CPU: u64 = 0x0002_0000; // 128KB

/// Virtio MMIO device region base address.
#[cfg(target_arch = "aarch64")]
pub(super) const VIRTIO_MMIO_BASE: u64 = 0x0A00_0000;

/// First SPI number for virtio devices (SPI 16 = global IRQ 48).
#[cfg(target_arch = "aarch64")]
pub(super) const VIRTIO_MMIO_IRQ_BASE: u32 = 48;

/// Guest RAM base address (1 GiB mark).
#[cfg(target_arch = "aarch64")]
pub(super) const RAM_BASE: u64 = 0x4000_0000;

/// ARM64 kernel Image text_offset (standard for Image format).
#[cfg(target_arch = "aarch64")]
pub(super) const KERNEL_TEXT_OFFSET: u64 = 0x0008_0000;

/// Get the MMIO base address for virtio device at the given slot index.
#[cfg(target_arch = "aarch64")]
pub(super) const fn virtio_mmio_addr(slot: u32) -> u64 {
    VIRTIO_MMIO_BASE + (slot as u64) * VIRTIO_MMIO_SIZE
}

/// Get the IRQ number for virtio device at the given slot index.
#[cfg(target_arch = "aarch64")]
pub(super) const fn virtio_mmio_irq(slot: u32) -> u32 {
    VIRTIO_MMIO_IRQ_BASE + slot
}

// ---------------------------------------------------------------------------
// x86_64 guest physical address map
// ---------------------------------------------------------------------------

/// Guest RAM starts at physical address 0 on x86_64.
#[cfg(target_arch = "x86_64")]
pub(super) const RAM_BASE: u64 = 0;

/// Start of the conventional x86 PCI/MMIO hole.
#[cfg(target_arch = "x86_64")]
pub(super) const PCI_HOLE_START: u64 = 0xC000_0000; // 3 GiB
/// End of the conventional x86 PCI/MMIO hole.
#[cfg(target_arch = "x86_64")]
pub(super) const PCI_HOLE_END: u64 = 0x1_0000_0000; // 4 GiB
#[cfg(target_arch = "x86_64")]
pub(super) const PCI_HOLE_SIZE: u64 = PCI_HOLE_END - PCI_HOLE_START;

/// Protected-mode kernel entry point (standard bzImage load address).
#[cfg(target_arch = "x86_64")]
pub(super) const KERNEL_LOAD_ADDR: u64 = 0x10_0000; // 1 MiB

/// Boot parameters (zero page) address.
#[cfg(target_arch = "x86_64")]
pub(super) const BOOT_PARAMS_ADDR: u64 = 0x7000;

/// Kernel command line address.
#[cfg(target_arch = "x86_64")]
pub(super) const CMDLINE_ADDR: u64 = 0x2_0000;

/// Maximum kernel command line length.
#[cfg(target_arch = "x86_64")]
pub(super) const CMDLINE_MAX_SIZE: u64 = 4096;

/// GDT address (3 entries: null, code64, data).
#[cfg(target_arch = "x86_64")]
pub(super) const GDT_ADDR: u64 = 0x500;

/// PML4 page table address.
#[cfg(target_arch = "x86_64")]
pub(super) const PML4_ADDR: u64 = 0x9000;
/// PDPT page table address.
#[cfg(target_arch = "x86_64")]
pub(super) const PDPT_ADDR: u64 = 0xA000;
/// PD page table address.
#[cfg(target_arch = "x86_64")]
pub(super) const PD_ADDR: u64 = 0xB000;

/// Virtio MMIO base address inside the reserved x86 PCI/MMIO hole.
#[cfg(target_arch = "x86_64")]
pub(super) const VIRTIO_MMIO_BASE: u64 = 0xD000_0000;

/// First IRQ for virtio devices (above legacy ISA IRQs 0-4).
#[cfg(target_arch = "x86_64")]
pub(super) const VIRTIO_MMIO_IRQ_BASE: u32 = 5;

/// Get the MMIO base address for virtio device at the given slot index.
#[cfg(target_arch = "x86_64")]
pub(super) const fn virtio_mmio_addr(slot: u32) -> u64 {
    VIRTIO_MMIO_BASE + (slot as u64) * VIRTIO_MMIO_SIZE
}

/// Get the IRQ number for virtio device at the given slot index.
#[cfg(target_arch = "x86_64")]
pub(super) const fn virtio_mmio_irq(slot: u32) -> u32 {
    VIRTIO_MMIO_IRQ_BASE + slot
}

// ---------------------------------------------------------------------------
// E820 memory map (x86_64)
// ---------------------------------------------------------------------------

/// E820 memory type: usable RAM.
#[cfg(target_arch = "x86_64")]
pub(super) const E820_RAM: u32 = 1;
/// E820 memory type: reserved.
#[cfg(target_arch = "x86_64")]
pub(super) const E820_RESERVED: u32 = 2;

/// End of conventional memory (640 KiB) -- start of ISA hole.
#[cfg(target_arch = "x86_64")]
pub(super) const EBDA_START: u64 = 0x9_FC00;
/// End of ISA hole / start of high memory (1 MiB).
#[cfg(target_arch = "x86_64")]
pub(super) const HIGH_MEM_START: u64 = 0x10_0000;

/// ACPI Root System Description Pointer location.
///
/// Linux searches the first KiB of EBDA for the RSDP. Keep all synthetic ACPI
/// tables in the reserved EBDA/ISA-hole range so they never collide with RAM,
/// the kernel image, or boot_params.
#[cfg(target_arch = "x86_64")]
pub(super) const ACPI_RSDP_ADDR: u64 = EBDA_START;
#[cfg(target_arch = "x86_64")]
pub(super) const ACPI_RSDT_ADDR: u64 = EBDA_START + 0x20;
#[cfg(target_arch = "x86_64")]
pub(super) const ACPI_MADT_ADDR: u64 = EBDA_START + 0x100;
#[cfg(target_arch = "x86_64")]
pub(super) const BDA_EBDA_SEGMENT_ADDR: u64 = 0x040E;
#[cfg(target_arch = "x86_64")]
pub(super) const BIOS_RSDP_ADDR: u64 = 0xF0000;

/// Local APIC and IOAPIC physical addresses used by KVM's in-kernel irqchip.
#[cfg(target_arch = "x86_64")]
pub(super) const LOCAL_APIC_ADDR: u32 = 0xFEE0_0000;
#[cfg(target_arch = "x86_64")]
pub(super) const IO_APIC_ADDR: u32 = 0xFEC0_0000;

#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct KvmMemoryRegion {
    pub slot: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub host_offset: u64,
}

/// E820 table entry.
#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct E820Entry {
    pub addr: u64,
    pub size: u64,
    pub type_: u32,
}

/// Build E820 memory map for the given RAM size.
/// Returns entries with the standard ISA hole and, for guests above 3 GiB,
/// a PCI/MMIO hole from 3 GiB to 4 GiB.
#[cfg(target_arch = "x86_64")]
pub(super) fn build_e820_map(ram_size: u64) -> Vec<E820Entry> {
    let mut entries = Vec::with_capacity(5);
    // Low memory: 0 to 640K
    entries.push(E820Entry {
        addr: 0,
        size: EBDA_START,
        type_: E820_RAM,
    });
    // ISA hole: 640K to 1M (reserved)
    entries.push(E820Entry {
        addr: EBDA_START,
        size: HIGH_MEM_START - EBDA_START,
        type_: E820_RESERVED,
    });
    if ram_size <= HIGH_MEM_START {
        return entries;
    }

    let low_high_end = ram_size.min(PCI_HOLE_START);
    if low_high_end > HIGH_MEM_START {
        entries.push(E820Entry {
            addr: HIGH_MEM_START,
            size: low_high_end - HIGH_MEM_START,
            type_: E820_RAM,
        });
    }

    if ram_size > PCI_HOLE_START {
        entries.push(E820Entry {
            addr: PCI_HOLE_START,
            size: PCI_HOLE_SIZE,
            type_: E820_RESERVED,
        });
        entries.push(E820Entry {
            addr: PCI_HOLE_END,
            size: ram_size - PCI_HOLE_START,
            type_: E820_RAM,
        });
    }
    entries
}

#[cfg(target_arch = "x86_64")]
pub(super) fn guest_phys_end(ram_size: u64) -> u64 {
    if ram_size > PCI_HOLE_START {
        ram_size + PCI_HOLE_SIZE
    } else {
        ram_size
    }
}

#[cfg(target_arch = "x86_64")]
pub(super) fn gpa_to_ram_offset(gpa: u64, ram_size: u64) -> Option<u64> {
    let offset = if gpa < PCI_HOLE_START {
        gpa
    } else if gpa >= PCI_HOLE_END {
        gpa.checked_sub(PCI_HOLE_SIZE)?
    } else {
        return None;
    };
    (offset < ram_size).then_some(offset)
}

#[cfg(target_arch = "x86_64")]
pub(super) fn kvm_memory_regions(ram_size: u64) -> Vec<KvmMemoryRegion> {
    if ram_size <= PCI_HOLE_START {
        return vec![KvmMemoryRegion {
            slot: 0,
            guest_phys_addr: 0,
            memory_size: ram_size,
            host_offset: 0,
        }];
    }

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
            memory_size: ram_size - PCI_HOLE_START,
            host_offset: PCI_HOLE_START,
        },
    ]
}

/// Align a value up to the next page boundary.
pub(super) const fn page_align_up(val: u64) -> u64 {
    (val + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// Align a value down to the previous page boundary.
pub(super) const fn page_align_down(val: u64) -> u64 {
    val & !(PAGE_SIZE - 1)
}

// ---------------------------------------------------------------------------
// GuestMemory: mmap'd anonymous region for guest RAM
// ---------------------------------------------------------------------------

/// Owned guest memory region backed by anonymous mmap.
pub(super) struct GuestMemory {
    ptr: *mut u8,
    size: u64,
}

// Safety: the memory region is a plain anonymous mmap, usable from any thread.
unsafe impl Send for GuestMemory {}
unsafe impl Sync for GuestMemory {}

impl GuestMemory {
    /// Allocate a new guest memory region of the given size.
    /// The region is zero-initialized and page-aligned.
    pub fn new(size: u64) -> Result<Self> {
        if size == 0 || size % PAGE_SIZE != 0 {
            bail!("guest memory size must be non-zero and page-aligned, got {size}");
        }

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            bail!(
                "mmap guest memory ({size} bytes): {}",
                std::io::Error::last_os_error()
            );
        }

        Ok(Self {
            ptr: ptr as *mut u8,
            size,
        })
    }

    /// Base host pointer for KVM_SET_USER_MEMORY_REGION.
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    pub fn as_ptr_at(&self, offset: u64) -> Result<*const u8> {
        if offset > self.size {
            bail!("guest memory pointer offset out of bounds: offset={offset:#x}");
        }
        Ok(unsafe { self.ptr.add(offset as usize) })
    }

    /// Size of the guest memory region.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Write bytes into guest memory at a given offset from RAM_BASE.
    /// The offset is relative to the start of the mmap'd region (i.e., guest
    /// physical address = RAM_BASE + offset).
    pub fn write_at(&self, offset: u64, data: &[u8]) -> Result<()> {
        let end = offset + data.len() as u64;
        if end > self.size {
            bail!(
                "guest memory write out of bounds: offset={offset:#x}, len={}, size={:#x}",
                data.len(),
                self.size
            );
        }
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr.add(offset as usize), data.len());
        }
        Ok(())
    }

    #[cfg(target_arch = "x86_64")]
    pub fn write_gpa(&self, gpa: u64, data: &[u8]) -> Result<()> {
        let offset = gpa_to_ram_offset(gpa, self.size)
            .ok_or_else(|| anyhow::anyhow!("guest physical address not backed by RAM: {gpa:#x}"))?;
        self.write_at(offset, data)
    }

    /// Read bytes from guest memory at a given offset from RAM_BASE.
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let end = offset + buf.len() as u64;
        if end > self.size {
            bail!(
                "guest memory read out of bounds: offset={offset:#x}, len={}, size={:#x}",
                buf.len(),
                self.size
            );
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.ptr.add(offset as usize),
                buf.as_mut_ptr(),
                buf.len(),
            );
        }
        Ok(())
    }

    /// Get a host pointer to a guest memory offset (for virtqueue access).
    ///
    /// # Safety
    /// The caller must ensure the offset + len is within bounds and the
    /// returned pointer is not used after the GuestMemory is dropped.
    pub unsafe fn host_ptr(&self, offset: u64) -> *mut u8 {
        self.ptr.add(offset as usize)
    }

    /// Clone a reference to this guest memory (for passing to virtio devices).
    /// The underlying mmap is shared -- only one GuestMemory owns the mmap
    /// and will unmap on drop. `ram_base` is the guest physical address where
    /// this memory region starts (architecture-dependent).
    pub fn clone_ref(&self, ram_base: u64) -> GuestMemoryRef {
        GuestMemoryRef {
            ptr: self.ptr,
            size: self.size,
            ram_base,
        }
    }
}

impl Drop for GuestMemory {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                libc::munmap(self.ptr as *mut libc::c_void, self.size as usize);
            }
        }
    }
}

/// Non-owning reference to guest memory (does not unmap on drop).
#[derive(Clone)]
pub(super) struct GuestMemoryRef {
    ptr: *mut u8,
    size: u64,
    ram_base: u64,
}

unsafe impl Send for GuestMemoryRef {}
unsafe impl Sync for GuestMemoryRef {}

impl GuestMemoryRef {
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Convert a guest physical address to a host pointer.
    /// Returns None if the address is outside the RAM region.
    pub fn gpa_to_host(&self, gpa: u64) -> Option<*mut u8> {
        #[cfg(target_arch = "x86_64")]
        {
            let offset = gpa_to_ram_offset(gpa, self.size)?;
            Some(unsafe { self.ptr.add(offset as usize) })
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            let offset = gpa.checked_sub(self.ram_base)?;
            if offset >= self.size {
                return None;
            }
            Some(unsafe { self.ptr.add(offset as usize) })
        }
    }

    pub fn write_at(&self, offset: u64, data: &[u8]) -> Result<()> {
        let end = offset + data.len() as u64;
        if end > self.size {
            bail!("guest memory write out of bounds");
        }
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr.add(offset as usize), data.len());
        }
        Ok(())
    }

    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let end = offset + buf.len() as u64;
        if end > self.size {
            bail!("guest memory read out of bounds");
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.ptr.add(offset as usize),
                buf.as_mut_ptr(),
                buf.len(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // aarch64 address map constants
    // -----------------------------------------------------------------------

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn gic_below_ram() {
        assert!(GIC_DIST_BASE + GIC_DIST_SIZE <= RAM_BASE);
        assert!(GIC_REDIST_BASE < RAM_BASE);
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
    fn guest_memory_ref_write_read() {
        let mem = GuestMemory::new(4096).unwrap();
        let memref = mem.clone_ref(RAM_BASE);

        memref.write_at(0, b"via ref").unwrap();
        let mut buf = vec![0u8; 7];
        memref.read_at(0, &mut buf).unwrap();
        assert_eq!(buf, b"via ref");
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
        assert!(initrd_start % PAGE_SIZE == 0);
        assert!(initrd_start + initrd_size <= ram_end);
        assert!(initrd_start > RAM_BASE + KERNEL_TEXT_OFFSET); // doesn't overlap kernel region
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn fdt_after_kernel_page_aligned() {
        let kernel_size: u64 = 30 * 1024 * 1024; // 30MB
        let kernel_end = RAM_BASE + KERNEL_TEXT_OFFSET + kernel_size;
        let fdt_start = page_align_up(kernel_end);

        assert!(fdt_start % PAGE_SIZE == 0);
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
    fn x86_64_kernel_above_legacy_hole() {
        assert!(KERNEL_LOAD_ADDR >= HIGH_MEM_START);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn x86_64_boot_structs_below_ebda() {
        assert!(BOOT_PARAMS_ADDR + 4096 <= EBDA_START);
        assert!(GDT_ADDR + 24 <= EBDA_START);
        assert!(PML4_ADDR + PAGE_SIZE <= EBDA_START);
        assert!(PDPT_ADDR + PAGE_SIZE <= EBDA_START);
        assert!(PD_ADDR + PAGE_SIZE <= EBDA_START);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
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
    fn x86_64_irq_base_above_legacy() {
        assert!(
            VIRTIO_MMIO_IRQ_BASE > 4,
            "must not conflict with ISA IRQs 0-4"
        );
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn aarch64_gic_spi_range_valid() {
        assert!(
            VIRTIO_MMIO_IRQ_BASE >= 32,
            "virtio IRQs must be in GIC SPI range (>=32)"
        );
        let max_irq = VIRTIO_MMIO_IRQ_BASE + VIRTIO_MMIO_MAX_DEVICES;
        assert!(
            max_irq < 1020,
            "virtio IRQs must stay within GIC SPI range (<1020)"
        );
    }
}
