//! OS confinement for the descriptor-only TCP publication companion.

use std::io;

/// Close every inherited descriptor except the three explicitly assigned
/// standard streams before creating runtime or parent-watch threads.
///
/// # Safety
/// Call only at process entry, before any other code owns descriptors above 2
/// or another thread can open/reuse them.
pub unsafe fn close_inherited_descriptors() -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let directory = "/dev/fd";
    #[cfg(not(target_os = "macos"))]
    let directory = "/proc/self/fd";
    let mut descriptors = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let name = entry?.file_name();
        let fd: i32 = name
            .to_str()
            .ok_or_else(|| io::Error::other("invalid descriptor name"))?
            .parse()
            .map_err(io::Error::other)?;
        if fd > 2 {
            descriptors.push(fd);
        }
    }
    for fd in descriptors {
        // SAFETY: the caller guarantees no live Rust owner or fd reuse. The
        // directory iterator itself has closed; EBADF for that fd is expected.
        if unsafe { libc::close(fd) } < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EBADF) {
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Restrict the whole process after runtime setup and descriptor inheritance.
/// The only network listener granted on macOS is this pre-bound loopback port.
pub fn confine(port: u16) -> io::Result<()> {
    if port == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "router listener must be bound",
        ));
    }
    platform::confine(port)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::{CStr, CString};

    unsafe extern "C" {
        fn sandbox_init(profile: *const libc::c_char, flags: u64, error: *mut *mut libc::c_char) -> libc::c_int;
        fn sandbox_free_error(error: *mut libc::c_char);
    }

    pub(super) fn confine(port: u16) -> io::Result<()> {
        let profile = CString::new(format!(
            "(version 1)(deny default)(allow network-inbound (local ip \"localhost:{port}\"))"
        ))?;
        let mut error = std::ptr::null_mut();
        // SAFETY: the profile is NUL terminated; sandbox_init initializes the
        // error pointer, whose allocation is released through its paired API.
        let status = unsafe { sandbox_init(profile.as_ptr(), 0, &mut error) };
        let message = if error.is_null() {
            "router sandbox initialization failed".into()
        } else {
            let message = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
            unsafe { sandbox_free_error(error) };
            message
        };
        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::other(message))
        }
    }
}

#[cfg(target_os = "linux")]
#[path = "router_sandbox/linux.rs"]
mod platform;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    pub(super) fn confine(_: u16) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "router sandbox unavailable",
        ))
    }
}

#[cfg(test)]
mod tests;
