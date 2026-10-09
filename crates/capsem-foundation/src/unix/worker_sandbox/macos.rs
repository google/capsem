use super::{Access, Policy};
use std::ffi::CStr;
use std::io;

unsafe extern "C" {
    fn sandbox_init_with_parameters(
        profile: *const libc::c_char,
        flags: u64,
        parameters: *const *const libc::c_char,
        error: *mut *mut libc::c_char,
    ) -> libc::c_int;
    fn sandbox_free_error(error: *mut libc::c_char);
}

pub(super) fn confine(policy: &Policy) -> io::Result<()> {
    validate_paths(policy)?;
    let compiled = super::seatbelt::compile(policy)?;
    let parameters = compiled.parameter_ptrs();
    let mut error = std::ptr::null_mut();
    // SAFETY: source and every alternating key/value parameter are NUL
    // terminated and remain alive for the synchronous initialization call.
    let status =
        unsafe { sandbox_init_with_parameters(compiled.source().as_ptr(), 0, parameters.as_ptr(), &mut error) };
    if status == 0 {
        return Ok(());
    }
    let message = if error.is_null() {
        "worker sandbox initialization failed".into()
    } else {
        // SAFETY: sandbox_init initialized this error allocation; its paired
        // API releases it after the bytes have been copied.
        let message = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
        unsafe { sandbox_free_error(error) };
        message
    };
    Err(io::Error::other(message))
}

fn validate_paths(policy: &Policy) -> io::Result<()> {
    for rule in policy.paths() {
        let metadata = std::fs::symlink_metadata(rule.path())?;
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("sandbox grant is a symlink: {}", rule.path().display()),
            ));
        }
        if rule.access() == Access::Executable && !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("sandbox executable grant is not a file: {}", rule.path().display()),
            ));
        }
    }
    Ok(())
}
