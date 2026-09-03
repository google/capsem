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
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
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
            bail!("mmap guest memory ({size} bytes): {}", std::io::Error::last_os_error());
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
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("guest memory write offset overflow"))?;
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
        let end = offset
            .checked_add(buf.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("guest memory read offset overflow"))?;
        if end > self.size {
            bail!(
                "guest memory read out of bounds: offset={offset:#x}, len={}, size={:#x}",
                buf.len(),
                self.size
            );
        }
        unsafe {
            std::ptr::copy_nonoverlapping(self.ptr.add(offset as usize), buf.as_mut_ptr(), buf.len());
        }
        Ok(())
    }

    /// Get a host pointer to a guest memory offset (for virtqueue access).
    ///
    /// # Safety
    /// The caller must ensure the offset + len is within bounds and the
    /// returned pointer is not used after the GuestMemory is dropped.
    pub unsafe fn host_ptr(&self, offset: u64) -> *mut u8 {
        // SAFETY: The caller owns the offset-in-bounds obligation documented
        // above; this block keeps that obligation visible at the operation.
        unsafe { self.ptr.add(offset as usize) }
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

    /// Convert a complete guest physical range to a host pointer.
    ///
    /// This is stricter than `gpa_to_host`: callers that expose guest memory to
    /// host syscalls must prove the whole range is backed by one contiguous RAM
    /// span, not just that the first byte has a valid translation.
    pub fn gpa_range_to_host(&self, gpa: u64, len: u64) -> Option<*mut u8> {
        if len == 0 {
            return self.gpa_to_host(gpa);
        }

        let last_gpa = gpa.checked_add(len.checked_sub(1)?)?;

        #[cfg(target_arch = "x86_64")]
        {
            let start_offset = gpa_to_ram_offset(gpa, self.size)?;
            let last_offset = gpa_to_ram_offset(last_gpa, self.size)?;
            if last_offset.checked_sub(start_offset)? != len - 1 {
                return None;
            }
            Some(unsafe { self.ptr.add(start_offset as usize) })
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            let start_offset = gpa.checked_sub(self.ram_base)?;
            let last_offset = last_gpa.checked_sub(self.ram_base)?;
            if last_offset >= self.size || last_offset.checked_sub(start_offset)? != len - 1 {
                return None;
            }
            Some(unsafe { self.ptr.add(start_offset as usize) })
        }
    }

    pub fn write_at(&self, offset: u64, data: &[u8]) -> Result<()> {
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("guest memory write offset overflow"))?;
        if end > self.size {
            bail!("guest memory write out of bounds");
        }
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr.add(offset as usize), data.len());
        }
        Ok(())
    }

    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let end = offset
            .checked_add(buf.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("guest memory read offset overflow"))?;
        if end > self.size {
            bail!("guest memory read out of bounds");
        }
        unsafe {
            std::ptr::copy_nonoverlapping(self.ptr.add(offset as usize), buf.as_mut_ptr(), buf.len());
        }
        Ok(())
    }

    /// Copy `data` into the guest buffer at `gpa`, but only if the entire
    /// destination range is backed by one contiguous RAM span. Returns the
    /// number of bytes written (0 if the range escapes RAM). Device write
    /// paths (FUSE responses, console TX) MUST route through this instead of
    /// translating only the first byte with `gpa_to_host` and then copying a
    /// guest-controlled length -- that is a guest->host OOB write primitive.
    pub fn write_guest_buffer(&self, gpa: u64, data: &[u8]) -> usize {
        if data.is_empty() {
            return 0;
        }
        match self.gpa_range_to_host(gpa, data.len() as u64) {
            Some(ptr) => {
                // SAFETY: gpa_range_to_host proved the whole [gpa, gpa+len)
                // range is backed by RAM, so `ptr..ptr+len` is in bounds.
                unsafe {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                }
                data.len()
            }
            None => 0,
        }
    }

    /// Append `len` bytes from the guest buffer at `gpa` to `dst`, but only if
    /// the entire source range is backed by one contiguous RAM span. Returns
    /// `false` (appending nothing) if the range escapes RAM. Device read paths
    /// (FUSE request gather) MUST route through this rather than `gpa_to_host`
    /// plus a guest-controlled length, which reads host memory past the mmap.
    pub fn read_guest_buffer(&self, gpa: u64, len: usize, dst: &mut Vec<u8>) -> bool {
        if len == 0 {
            return true;
        }
        match self.gpa_range_to_host(gpa, len as u64) {
            Some(ptr) => {
                // SAFETY: gpa_range_to_host proved the whole [gpa, gpa+len)
                // range is backed by RAM, so reading `len` bytes is in bounds.
                dst.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr.cast_const(), len) });
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests;
