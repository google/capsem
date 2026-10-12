//! Lazy anonymous guest RAM with aligned, optional transparent huge pages.

use std::ffi::c_void;
use std::io;
use std::num::NonZeroUsize;
use std::ptr::NonNull;
use std::sync::OnceLock;

use nix::sys::mman::{madvise, mmap_anonymous, munmap, MapFlags, MmapAdvise, ProtFlags};

const HUGE_PAGE_SIZE: usize = 2 * 1024 * 1024;

/// Host PMD alignment: at least two MiB, also covering 64-KiB-page ARM hosts.
pub fn page_alignment() -> usize {
    static ALIGNMENT: OnceLock<usize> = OnceLock::new();
    *ALIGNMENT.get_or_init(|| {
        super::fs::read_regular_file_no_follow(std::path::Path::new(
            "/sys/kernel/mm/transparent_hugepage/hpage_pmd_size",
        ))
        .ok()
        .and_then(|bytes| std::str::from_utf8(&bytes).ok().and_then(parse_alignment))
        .unwrap_or(HUGE_PAGE_SIZE)
    })
}

fn parse_alignment(text: &str) -> Option<usize> {
    let value: usize = text.trim().parse().ok()?;
    (value.is_power_of_two() && (HUGE_PAGE_SIZE..=1024 * 1024 * 1024).contains(&value)).then_some(value)
}

/// Owns the whole mapping, including a virtual-only alignment margin.
/// Exposes no references; callers must bound and synchronize pointer accesses.
pub struct GuestRam {
    mapping: NonNull<c_void>,
    mapping_len: usize,
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: ownership may cross threads; accesses through pointers remain unsafe.
unsafe impl Send for GuestRam {}
unsafe impl Sync for GuestRam {}

impl GuestRam {
    pub fn new(len: usize) -> io::Result<Self> {
        if len == 0 || !len.is_multiple_of(4096) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "guest RAM must contain complete pages",
            ));
        }
        let alignment = page_alignment();
        let mapping_len = len
            .checked_add(alignment)
            .and_then(NonZeroUsize::new)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "guest RAM length overflows"))?;
        // SAFETY: creates a fresh, zeroed anonymous mapping with no file alias.
        let mapping = unsafe {
            mmap_anonymous(
                None,
                mapping_len,
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_PRIVATE | MapFlags::MAP_NORESERVE,
            )
        }
        .map_err(super::errno::io)?;
        let offset = mapping.as_ptr().addr().wrapping_neg() & (alignment - 1);
        // SAFETY: offset is less than the reserved margin; len bytes remain mapped.
        let ptr = unsafe { NonNull::new_unchecked(mapping.cast::<u8>().as_ptr().add(offset)) };
        Ok(Self {
            mapping,
            mapping_len: mapping_len.get(),
            ptr,
            len,
        })
    }

    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// A per-mapping hint, with no global host policy changes or eager touching.
    /// Callers may keep ordinary pages if the kernel cannot honor the hint.
    pub fn prefer_huge_pages(&self) -> io::Result<()> {
        // SAFETY: this is the aligned payload within our owned live mapping.
        unsafe { madvise(self.ptr.cast(), self.len, MmapAdvise::MADV_HUGEPAGE) }.map_err(super::errno::io)
    }

    /// Reset an inactive VM's RAM to zero without touching or allocating pages.
    /// Pointer users must be quiescent, just as during checkpoint restoration.
    pub fn discard_contents(&self) -> io::Result<()> {
        // SAFETY: the complete payload is live, anonymous, private and aligned.
        unsafe { madvise(self.ptr.cast(), self.len, MmapAdvise::MADV_DONTNEED) }.map_err(super::errno::io)
    }
}

impl Drop for GuestRam {
    fn drop(&mut self) {
        // SAFETY: this object is the unique owner of the exact original mapping.
        let _ = unsafe { munmap(self.mapping, self.mapping_len) };
    }
}

#[cfg(test)]
mod tests;
