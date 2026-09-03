//! Sparse-preserving file copy used when a reflink is unavailable on Linux.

use std::path::Path;

#[cfg(target_os = "linux")]
pub(crate) fn copy_file_sparse(src: &Path, dst: &Path) -> std::io::Result<()> {
    let mut input = std::fs::File::open(src)?;
    let metadata = input.metadata()?;
    let mut output = std::fs::File::create(dst)?;

    match copy_sparse_extents(&mut input, &mut output, metadata.len()) {
        Ok(()) => {}
        Err(err) if seek_data_unsupported(&err) => {
            copy_sparse_by_scanning(&mut input, &mut output)?;
        }
        Err(err) => return Err(err),
    }

    output.set_len(metadata.len())?;
    output.set_permissions(metadata.permissions())?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn copy_sparse_extents(input: &mut std::fs::File, output: &mut std::fs::File, logical_len: u64) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::unix::io::AsRawFd;

    // Ext4 system-overlay files often contain large allocated extents with
    // only a few non-zero filesystem blocks. Writing one whole MiB whenever
    // any byte in it is non-zero turns tiny layout shifts into MiB-sized fork
    // regressions. Scan allocated extents at the ordinary filesystem block
    // granularity; SEEK_DATA/SEEK_HOLE already keeps us away from true holes.
    const CHUNK_SIZE: usize = 4096;

    let mut offset = 0_u64;
    let mut buffer = vec![0_u8; CHUNK_SIZE];
    let zero_buffer = vec![0_u8; CHUNK_SIZE];

    while offset < logical_len {
        let data = match lseek_extent(input.as_raw_fd(), offset, libc::SEEK_DATA) {
            Ok(data) => data,
            Err(err) if err.raw_os_error() == Some(libc::ENXIO) => break,
            Err(err) => return Err(err),
        };
        if data >= logical_len {
            break;
        }

        let hole = match lseek_extent(input.as_raw_fd(), data, libc::SEEK_HOLE) {
            Ok(hole) => hole.min(logical_len),
            Err(err) if err.raw_os_error() == Some(libc::ENXIO) => logical_len,
            Err(err) => return Err(err),
        };
        if hole <= data {
            offset = data + 1;
            continue;
        }

        input.seek(SeekFrom::Start(data))?;
        output.seek(SeekFrom::Start(data))?;
        let mut remaining = hole - data;
        while remaining > 0 {
            let read_len = (remaining as usize).min(buffer.len());
            let read = input.read(&mut buffer[..read_len])?;
            if read == 0 {
                break;
            }
            let chunk = &buffer[..read];
            if chunk_is_zero(chunk, &zero_buffer) {
                output.seek(SeekFrom::Current(read as i64))?;
            } else {
                output.write_all(chunk)?;
            }
            remaining -= read as u64;
        }

        offset = hole;
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn lseek_extent(fd: std::os::unix::io::RawFd, offset: u64, whence: libc::c_int) -> std::io::Result<u64> {
    let result = unsafe { libc::lseek(fd, offset as libc::off_t, whence) };
    if result < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(result as u64)
    }
}

#[cfg(target_os = "linux")]
fn seek_data_unsupported(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS | libc::ENOTTY))
}

#[cfg(target_os = "linux")]
fn copy_sparse_by_scanning(input: &mut std::fs::File, output: &mut std::fs::File) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};

    const CHUNK_SIZE: usize = 1024 * 1024;

    input.seek(SeekFrom::Start(0))?;
    output.set_len(0)?;
    output.seek(SeekFrom::Start(0))?;

    let mut buffer = vec![0_u8; CHUNK_SIZE];
    let zero_buffer = vec![0_u8; CHUNK_SIZE];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        if chunk_is_zero(chunk, &zero_buffer) {
            output.seek(SeekFrom::Current(read as i64))?;
        } else {
            output.write_all(chunk)?;
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn chunk_is_zero(chunk: &[u8], zero_buffer: &[u8]) -> bool {
    debug_assert!(chunk.len() <= zero_buffer.len());
    if chunk.is_empty() {
        return true;
    }
    unsafe { libc::memcmp(chunk.as_ptr().cast(), zero_buffer.as_ptr().cast(), chunk.len()) == 0 }
}
