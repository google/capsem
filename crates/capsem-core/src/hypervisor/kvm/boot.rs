//! Kernel and initrd loading into guest memory.
//!
//! Handles ARM64 Image format header parsing, kernel/initrd/FDT placement,
//! and initial vCPU register setup.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::memory::{self, GuestMemory};
use super::sys;

// ---------------------------------------------------------------------------
// ARM64 kernel Image header
// ---------------------------------------------------------------------------

/// ARM64 Image magic number at offset 56: "ARM\x64" (little-endian: 0x644d5241).
const ARM64_IMAGE_MAGIC: u32 = 0x644d5241;

/// Offset of the magic number within the ARM64 Image header.
const MAGIC_OFFSET: usize = 56;

/// Offset of text_offset field within the ARM64 Image header (bytes 8-15).
const TEXT_OFFSET_FIELD: usize = 8;

/// Result of loading a kernel image.
#[derive(Debug)]
pub(super) struct KernelLoadInfo {
    /// Guest physical address where the kernel entry point is.
    pub entry_addr: u64,
    /// Guest physical address of the first byte after the kernel.
    pub kernel_end: u64,
}

/// Result of loading an initrd.
#[derive(Debug)]
pub(super) struct InitrdLoadInfo {
    /// Guest physical address where the initrd was loaded.
    pub guest_addr: u64,
    /// Size of the initrd in bytes.
    pub size: usize,
}

/// Parse an ARM64 Image header and extract the text_offset.
///
/// Returns the text_offset (bytes 8-15 of the header). If the header
/// lacks the ARM64 magic or is too short, falls back to the standard
/// offset (0x80000).
fn parse_arm64_header(data: &[u8]) -> u64 {
    if data.len() < 64 {
        return memory::KERNEL_TEXT_OFFSET;
    }

    // Check magic at offset 56
    let magic = u32::from_le_bytes([
        data[MAGIC_OFFSET],
        data[MAGIC_OFFSET + 1],
        data[MAGIC_OFFSET + 2],
        data[MAGIC_OFFSET + 3],
    ]);

    if magic != ARM64_IMAGE_MAGIC {
        return memory::KERNEL_TEXT_OFFSET;
    }

    // Extract text_offset from bytes 8-15 (little-endian u64)
    let text_offset = u64::from_le_bytes([
        data[TEXT_OFFSET_FIELD],
        data[TEXT_OFFSET_FIELD + 1],
        data[TEXT_OFFSET_FIELD + 2],
        data[TEXT_OFFSET_FIELD + 3],
        data[TEXT_OFFSET_FIELD + 4],
        data[TEXT_OFFSET_FIELD + 5],
        data[TEXT_OFFSET_FIELD + 6],
        data[TEXT_OFFSET_FIELD + 7],
    ]);

    // A text_offset of 0 means the default (0x80000) per the ARM64 boot protocol.
    // Extremely large values are also suspicious.
    if text_offset == 0 || text_offset > 256 * 1024 * 1024 {
        return memory::KERNEL_TEXT_OFFSET;
    }

    text_offset
}

/// Load a kernel Image into guest memory.
///
/// The kernel is loaded at RAM_BASE + text_offset. Returns the entry point
/// address and the end address (for placing the FDT after it).
pub(super) fn load_kernel(mem: &GuestMemory, kernel_path: &Path) -> Result<KernelLoadInfo> {
    let kernel_data = std::fs::read(kernel_path)
        .with_context(|| format!("reading kernel: {}", kernel_path.display()))?;

    if kernel_data.is_empty() {
        bail!("kernel image is empty: {}", kernel_path.display());
    }

    // Reject bzImage (x86_64) kernels -- HdrS magic at offset 0x202
    if kernel_data.len() > 0x206 {
        let hdrs = u32::from_le_bytes([
            kernel_data[0x202],
            kernel_data[0x203],
            kernel_data[0x204],
            kernel_data[0x205],
        ]);
        if hdrs == 0x5372_6448 {
            bail!("kernel is a bzImage (x86_64) but this is an aarch64 host");
        }
    }

    let text_offset = parse_arm64_header(&kernel_data);
    let load_offset = text_offset; // offset within guest RAM
    let entry_addr = memory::RAM_BASE + text_offset;

    // Check kernel fits in RAM
    let kernel_end_offset = load_offset + kernel_data.len() as u64;
    if kernel_end_offset > mem.size() {
        bail!(
            "kernel too large for guest memory: kernel needs {:#x} bytes at offset {:#x}, RAM is {:#x}",
            kernel_data.len(),
            load_offset,
            mem.size()
        );
    }

    mem.write_at(load_offset, &kernel_data)
        .context("writing kernel to guest memory")?;

    Ok(KernelLoadInfo {
        entry_addr,
        kernel_end: memory::RAM_BASE + kernel_end_offset,
    })
}

/// Load an initrd into guest memory at the end of RAM (page-aligned down).
///
/// Placing the initrd at the end of RAM avoids overlapping with the kernel's
/// BSS expansion. The start and end addresses are recorded in the FDT.
pub(super) fn load_initrd(
    mem: &GuestMemory,
    initrd_path: &Path,
    kernel_end: u64,
) -> Result<InitrdLoadInfo> {
    let initrd_data = std::fs::read(initrd_path)
        .with_context(|| format!("reading initrd: {}", initrd_path.display()))?;

    if initrd_data.is_empty() {
        bail!("initrd is empty: {}", initrd_path.display());
    }

    let ram_size = mem.size();
    let ram_end = memory::RAM_BASE + ram_size;
    let initrd_start = memory::page_align_down(ram_end - initrd_data.len() as u64);
    let offset = initrd_start - memory::RAM_BASE;

    if initrd_start < kernel_end {
        bail!(
            "initrd ({} bytes) too large to fit after kernel in {} bytes of RAM",
            initrd_data.len(),
            ram_size
        );
    }

    mem.write_at(offset, &initrd_data)
        .context("writing initrd to guest memory")?;

    Ok(InitrdLoadInfo {
        guest_addr: initrd_start,
        size: initrd_data.len(),
    })
}

/// Load an FDT blob into guest memory after the kernel.
///
/// The FDT is placed at the next page boundary after kernel_end.
/// Returns the guest physical address of the FDT.
pub(super) fn load_fdt(mem: &GuestMemory, fdt_blob: &[u8], kernel_end: u64) -> Result<u64> {
    let fdt_start = memory::page_align_up(kernel_end);
    let offset = fdt_start - memory::RAM_BASE;
    let fdt_end_offset = offset + fdt_blob.len() as u64;

    if fdt_end_offset > mem.size() {
        bail!(
            "FDT does not fit in guest memory: needs {:#x}, available {:#x}",
            fdt_end_offset,
            mem.size()
        );
    }

    // ARM64 boot protocol: FDT must be within 512MB of kernel entry
    let kernel_entry = memory::RAM_BASE + memory::KERNEL_TEXT_OFFSET;
    if fdt_start - kernel_entry > 512 * 1024 * 1024 {
        bail!("FDT at {fdt_start:#x} is more than 512MB from kernel entry at {kernel_entry:#x}");
    }

    mem.write_at(offset, fdt_blob)
        .context("writing FDT to guest memory")?;

    Ok(fdt_start)
}

/// Set initial boot registers on the primary vCPU (vCPU 0).
///
/// ARM64 boot protocol:
///   PC  = kernel entry point
///   X0  = FDT physical address
///   X1  = 0 (reserved)
///   X2  = 0 (reserved)
///   X3  = 0 (reserved)
pub(super) fn set_boot_regs(vcpu: &sys::VcpuFd, entry_addr: u64, fdt_addr: u64) -> Result<()> {
    vcpu.set_one_reg(sys::REG_PC, entry_addr)
        .context("setting PC")?;
    vcpu.set_one_reg(sys::REG_X0, fdt_addr)
        .context("setting X0 (FDT address)")?;
    vcpu.set_one_reg(sys::REG_X1, 0).context("setting X1")?;
    vcpu.set_one_reg(sys::REG_X2, 0).context("setting X2")?;
    vcpu.set_one_reg(sys::REG_X3, 0).context("setting X3")?;
    Ok(())
}

#[cfg(test)]
mod tests;
