//! Guest physical memory layout and management for aarch64 KVM.
//!
//! Defines the guest physical address map and provides a safe wrapper
//! around the mmap'd guest memory region.

use anyhow::{Result, bail};

// ---------------------------------------------------------------------------
// aarch64 guest physical address map
// ---------------------------------------------------------------------------

/// GIC distributor base address (64KB region).
pub(super) const GIC_DIST_BASE: u64 = 0x0800_0000;
/// GIC distributor region size.
pub(super) const GIC_DIST_SIZE: u64 = 0x0001_0000; // 64KB

/// GIC redistributor base address (128KB per vCPU).
pub(super) const GIC_REDIST_BASE: u64 = 0x080A_0000;
/// GIC redistributor size per vCPU.
pub(super) const GIC_REDIST_PER_CPU: u64 = 0x0002_0000; // 128KB

/// Virtio MMIO device region base address.
pub(super) const VIRTIO_MMIO_BASE: u64 = 0x0A00_0000;
/// Size of each virtio MMIO device slot.
pub(super) const VIRTIO_MMIO_SIZE: u64 = 0x200;
/// Maximum number of virtio MMIO device slots.
pub(super) const VIRTIO_MMIO_MAX_DEVICES: u32 = 32;

/// First SPI number for virtio devices (SPI 16 = global IRQ 48).
pub(super) const VIRTIO_MMIO_IRQ_BASE: u32 = 48;

/// Guest RAM base address (1 GiB mark).
pub(super) const RAM_BASE: u64 = 0x4000_0000;

/// ARM64 kernel Image text_offset (standard for Image format).
pub(super) const KERNEL_TEXT_OFFSET: u64 = 0x0008_0000;

/// Page size for alignment.
pub(super) const PAGE_SIZE: u64 = 4096;

/// Get the MMIO base address for virtio device at the given slot index.
pub(super) const fn virtio_mmio_addr(slot: u32) -> u64 {
    VIRTIO_MMIO_BASE + (slot as u64) * VIRTIO_MMIO_SIZE
}

/// Get the GIC SPI number (global IRQ) for virtio device at the given slot index.
pub(super) const fn virtio_mmio_irq(slot: u32) -> u32 {
    VIRTIO_MMIO_IRQ_BASE + slot
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
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.ptr.add(offset as usize),
                data.len(),
            );
        }
        Ok(())
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
    /// and will unmap on drop.
    pub fn clone_ref(&self) -> GuestMemoryRef {
        GuestMemoryRef {
            ptr: self.ptr,
            size: self.size,
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
        if gpa < RAM_BASE || gpa >= RAM_BASE + self.size {
            return None;
        }
        let offset = gpa - RAM_BASE;
        Some(unsafe { self.ptr.add(offset as usize) })
    }

    pub fn write_at(&self, offset: u64, data: &[u8]) -> Result<()> {
        let end = offset + data.len() as u64;
        if end > self.size {
            bail!("guest memory write out of bounds");
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.ptr.add(offset as usize),
                data.len(),
            );
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
    // Address map constants
    // -----------------------------------------------------------------------

    #[test]
    fn gic_below_ram() {
        assert!(GIC_DIST_BASE + GIC_DIST_SIZE <= RAM_BASE);
        assert!(GIC_REDIST_BASE < RAM_BASE);
    }

    #[test]
    fn virtio_mmio_below_ram() {
        let max_addr = virtio_mmio_addr(VIRTIO_MMIO_MAX_DEVICES - 1) + VIRTIO_MMIO_SIZE;
        assert!(max_addr <= RAM_BASE, "virtio MMIO region overlaps RAM");
    }

    #[test]
    fn gic_does_not_overlap_virtio() {
        let gic_end = GIC_REDIST_BASE + GIC_REDIST_PER_CPU * 8; // max 8 CPUs
        assert!(
            gic_end <= VIRTIO_MMIO_BASE || GIC_DIST_BASE >= VIRTIO_MMIO_BASE + VIRTIO_MMIO_SIZE * VIRTIO_MMIO_MAX_DEVICES as u64,
            "GIC and virtio MMIO regions overlap"
        );
    }

    #[test]
    fn virtio_mmio_addr_sequential() {
        assert_eq!(virtio_mmio_addr(0), VIRTIO_MMIO_BASE);
        assert_eq!(virtio_mmio_addr(1), VIRTIO_MMIO_BASE + 0x200);
        assert_eq!(virtio_mmio_addr(2), VIRTIO_MMIO_BASE + 0x400);
    }

    #[test]
    fn virtio_mmio_irq_sequential() {
        assert_eq!(virtio_mmio_irq(0), 48);
        assert_eq!(virtio_mmio_irq(1), 49);
        assert_eq!(virtio_mmio_irq(2), 50);
    }

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
        assert!(buf.iter().all(|&b| b == 0), "memory should be zero-initialized");
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
        let memref = mem.clone_ref();

        // Address within RAM region
        let ptr = memref.gpa_to_host(RAM_BASE);
        assert!(ptr.is_some());

        // Address before RAM base
        let ptr = memref.gpa_to_host(RAM_BASE - 1);
        assert!(ptr.is_none());

        // Address past end
        let ptr = memref.gpa_to_host(RAM_BASE + 4096);
        assert!(ptr.is_none());
    }

    #[test]
    fn guest_memory_ref_write_read() {
        let mem = GuestMemory::new(4096).unwrap();
        let memref = mem.clone_ref();

        memref.write_at(0, b"via ref").unwrap();
        let mut buf = vec![0u8; 7];
        memref.read_at(0, &mut buf).unwrap();
        assert_eq!(buf, b"via ref");
    }

    #[test]
    fn guest_memory_ref_shares_underlying_memory() {
        let mem = GuestMemory::new(4096).unwrap();
        let memref = mem.clone_ref();

        // Write via original
        mem.write_at(0, b"shared").unwrap();
        // Read via ref
        let mut buf = vec![0u8; 6];
        memref.read_at(0, &mut buf).unwrap();
        assert_eq!(buf, b"shared");
    }

    // -----------------------------------------------------------------------
    // Kernel/initrd placement calculations
    // -----------------------------------------------------------------------

    #[test]
    fn kernel_loads_at_correct_offset() {
        let kernel_addr = RAM_BASE + KERNEL_TEXT_OFFSET;
        assert_eq!(kernel_addr, 0x4008_0000);
    }

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
}
