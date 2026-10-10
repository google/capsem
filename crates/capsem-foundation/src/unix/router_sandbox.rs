//! OS confinement for the descriptor-only TCP publication companion.

use std::io;

pub use super::fd::close_inherited_descriptors;

/// Restrict the whole process after runtime setup and descriptor inheritance.
/// Only connected descriptors are granted; no listener permission is needed.
pub fn confine() -> io::Result<()> {
    platform::confine()
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::{CStr, CString};

    unsafe extern "C" {
        fn sandbox_init(profile: *const libc::c_char, flags: u64, error: *mut *mut libc::c_char) -> libc::c_int;
        fn sandbox_free_error(error: *mut libc::c_char);
    }

    pub(super) fn confine() -> io::Result<()> {
        let profile = CString::new("(version 1)(deny default)")?;
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
    pub(super) fn confine() -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "router sandbox unavailable",
        ))
    }
}

#[cfg(test)]
mod tests;
