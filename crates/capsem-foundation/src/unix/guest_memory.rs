//! Lazy anonymous guest RAM with aligned, optional transparent huge pages.

use std::ffi::c_void;
use std::io;
use std::num::NonZeroUsize;
use std::ptr::NonNull;

use nix::sys::mman::{madvise, mmap_anonymous, munmap, MapFlags, MmapAdvise, ProtFlags};

const HUGE_PAGE_SIZE: usize = 2 * 1024 * 1024;

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
        let mapping_len = len
            .checked_add(HUGE_PAGE_SIZE)
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
        let offset = mapping.as_ptr().addr().wrapping_neg() & (HUGE_PAGE_SIZE - 1);
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
}

impl Drop for GuestRam {
    fn drop(&mut self) {
        // SAFETY: this object is the unique owner of the exact original mapping.
        let _ = unsafe { munmap(self.mapping, self.mapping_len) };
    }
}

#[cfg(test)]
mod tests;
