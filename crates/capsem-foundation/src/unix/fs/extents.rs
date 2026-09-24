//! Copy-on-write file cloning with a sparse-preserving fallback.
//!
//! The source is an already-open descriptor, never a path: whatever the
//! caller opened (no-follow, regular-file-checked) is exactly what is cloned.
//! The destination is created exclusively below a [`ContainedDir`], so an
//! existing entry -- a planted symlink included -- is refused, not replaced.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, AsRawFd};

use nix::errno::Errno;

use super::super::contained::{ContainedDir, ContainedOpenOptions};
use super::super::errno;
use super::durability::set_mode;

/// How a clone's bytes came to exist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloneMethod {
    /// Extents shared with the source (APFS clonefile, Linux FICLONE).
    SharedExtents,
    /// The filesystem cannot share extents; written blocks were copied and
    /// holes and zero blocks left sparse.
    SparseCopy,
}

/// Clone the open regular file `source` into the new entry `name` of `dir`
/// with permission bits `mode`. The new file is returned open for reading.
pub fn clone_file_into(source: &File, dir: &ContainedDir, name: &OsStr, mode: u32) -> io::Result<(File, CloneMethod)> {
    platform::clone_file_into(source, dir, name, mode)
}

/// Copy `source` into the empty `dest`, writing only non-zero blocks, and
/// size `dest` to the source's length.
pub fn copy_sparse(source: &File, dest: &File) -> io::Result<()> {
    let len = source.metadata()?.len();
    platform::copy_blocks(source, dest, len)?;
    dest.set_len(len)
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    pub(super) fn clone_file_into(
        source: &File,
        dir: &ContainedDir,
        name: &OsStr,
        mode: u32,
    ) -> io::Result<(File, CloneMethod)> {
        let c_name = CString::new(name.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "name contains a NUL byte"))?;
        // SAFETY: both descriptors are open for the call and `c_name` is one
        // NUL-terminated component. fclonefileat clones from the source
        // descriptor, resolving no source path, and fails on any existing
        // destination entry. nix has no wrapper for it.
        let result = unsafe { libc::fclonefileat(source.as_raw_fd(), dir.as_fd().as_raw_fd(), c_name.as_ptr(), 0) };
        if result == 0 {
            let clone = dir.open_file(name, ContainedOpenOptions::read_only())?;
            set_mode(clone.as_fd(), mode)?;
            return Ok((clone, CloneMethod::SharedExtents));
        }
        match Errno::last() {
            Errno::ENOTSUP | Errno::EXDEV => {}
            error => return Err(errno::io(error)),
        }
        let clone = dir.open_file(name, ContainedOpenOptions::write_create_new(mode))?;
        copy_sparse(source, &clone)?;
        set_mode(clone.as_fd(), mode)?;
        Ok((clone, CloneMethod::SparseCopy))
    }

    /// No SEEK_DATA here: scan every block, still skipping zero ones.
    pub(super) fn copy_blocks(source: &File, dest: &File, _len: u64) -> io::Result<()> {
        super::copy_scanning(source, dest)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use nix::unistd::{lseek, Whence};

    use super::*;

    /// Filesystem block granularity: writing a whole MiB whenever one byte in
    /// it is non-zero turned small layout shifts into MiB-sized fork regressions.
    const EXTENT_CHUNK: usize = 4096;

    mod ioctl {
        // FICLONE is _IOW(0x94, 9, int): the source descriptor travels by value.
        nix::ioctl_write_int!(ficlone, 0x94, 9);
    }

    pub(super) fn clone_file_into(
        source: &File,
        dir: &ContainedDir,
        name: &OsStr,
        mode: u32,
    ) -> io::Result<(File, CloneMethod)> {
        let clone = dir.open_file(name, ContainedOpenOptions::write_create_new(mode))?;
        // SAFETY: both descriptors are open files for the duration of the call.
        let method = match unsafe { ioctl::ficlone(clone.as_raw_fd(), source.as_raw_fd() as _) } {
            Ok(_) => CloneMethod::SharedExtents,
            Err(Errno::EOPNOTSUPP | Errno::ENOSYS | Errno::EXDEV | Errno::EINVAL) => {
                copy_sparse(source, &clone)?;
                CloneMethod::SparseCopy
            }
            Err(error) => return Err(errno::io(error)),
        };
        set_mode(clone.as_fd(), mode)?;
        Ok((clone, method))
    }

    /// Read only allocated extents (SEEK_DATA/SEEK_HOLE): a mostly-empty
    /// multi-GiB overlay image costs its written blocks, not its length.
    pub(super) fn copy_blocks(source: &File, dest: &File, len: u64) -> io::Result<()> {
        if let Err(Errno::EINVAL | Errno::ENOSYS | Errno::ENOTTY) = seek(source, 0, Whence::SeekData) {
            return super::copy_scanning(source, dest);
        }
        let mut buffer = vec![0_u8; EXTENT_CHUNK];
        let mut offset = 0_u64;
        while offset < len {
            let data = match seek(source, offset, Whence::SeekData) {
                Ok(data) if data < len => data,
                Ok(_) | Err(Errno::ENXIO) => break,
                Err(error) => return Err(errno::io(error)),
            };
            let hole = match seek(source, data, Whence::SeekHole) {
                Ok(hole) => hole.min(len),
                Err(Errno::ENXIO) => len,
                Err(error) => return Err(errno::io(error)),
            };
            if hole <= data {
                offset = data + 1;
                continue;
            }
            super::seek_to(source, SeekFrom::Start(data))?;
            super::seek_to(dest, SeekFrom::Start(data))?;
            super::copy_run(source, dest, &mut buffer, hole - data)?;
            offset = hole;
        }
        Ok(())
    }

    fn seek(file: &File, offset: u64, whence: Whence) -> nix::Result<u64> {
        let offset = offset.try_into().map_err(|_| Errno::EOVERFLOW)?;
        let position = lseek(file.as_raw_fd(), offset, whence)?;
        u64::try_from(position).map_err(|_| Errno::EOVERFLOW)
    }
}

/// Chunk for a full scan when the filesystem cannot report extents.
const SCAN_CHUNK: usize = 1024 * 1024;

fn copy_scanning(source: &File, dest: &File) -> io::Result<()> {
    seek_to(source, SeekFrom::Start(0))?;
    dest.set_len(0)?;
    seek_to(dest, SeekFrom::Start(0))?;
    let mut buffer = vec![0_u8; SCAN_CHUNK];
    copy_run(source, dest, &mut buffer, u64::MAX)
}

fn seek_to(mut file: &File, position: SeekFrom) -> io::Result<u64> {
    file.seek(position)
}

/// Copy up to `remaining` bytes from the current offsets, seeking over zero
/// chunks instead of writing them.
fn copy_run(mut source: &File, mut dest: &File, buffer: &mut [u8], mut remaining: u64) -> io::Result<()> {
    while remaining > 0 {
        let want = usize::try_from(remaining).unwrap_or(usize::MAX).min(buffer.len());
        let read = source.read(&mut buffer[..want])?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        if chunk.iter().all(|byte| *byte == 0) {
            dest.seek(SeekFrom::Current(read as i64))?;
        } else {
            dest.write_all(chunk)?;
        }
        remaining -= read as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
