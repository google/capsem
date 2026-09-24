//! Descriptor-level mode and durability operations.
//!
//! Two flush strengths, named for what they promise. [`sync`] hands one file's
//! data and metadata to the device; [`sync_filesystem`] is the barrier after
//! which everything already synced on that filesystem survives power loss.
//! Syncing many files then one barrier is the cheap durable pattern: a full
//! barrier per file (what `File::sync_all` does on macOS) costs milliseconds each.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

use super::super::contained::permission_mode;
use super::super::errno;

/// Set permission bits on an open file or directory. Bits outside `0o7777`
/// are ignored.
pub fn set_mode(fd: BorrowedFd<'_>, mode: u32) -> io::Result<()> {
    nix::sys::stat::fchmod(fd.as_raw_fd(), permission_mode(mode)).map_err(errno::io)
}

/// Hand one file's data and metadata to the device (`fsync`), without the
/// drive-cache barrier. Pair with [`sync_filesystem`] for power-loss safety.
pub fn sync(fd: BorrowedFd<'_>) -> io::Result<()> {
    nix::unistd::fsync(fd.as_raw_fd()).map_err(errno::io)
}

/// Make one file covered by the next [`sync_filesystem`], at the platform's
/// cheapest cost: free on Linux, where syncfs flushes every dirty file; an
/// `fsync` on macOS, where the barrier covers only files already synced.
pub fn sync_before_barrier(fd: BorrowedFd<'_>) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let _ = fd;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        sync(fd)
    }
}

/// Make everything written on `fd`'s filesystem durable: `syncfs` on Linux;
/// on macOS, which has no syncfs, an `F_FULLFSYNC` drive-cache barrier that
/// covers every file already [`sync`]ed.
pub fn sync_filesystem(fd: BorrowedFd<'_>) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        nix::unistd::syncfs(fd.as_raw_fd()).map_err(errno::io)
    }
    #[cfg(target_os = "macos")]
    {
        use nix::fcntl::{fcntl, FcntlArg};
        fcntl(fd.as_raw_fd(), FcntlArg::F_FULLFSYNC)
            .map(drop)
            .map_err(errno::io)
    }
}

#[cfg(test)]
mod tests;
