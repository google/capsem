//! Conversion at the private nix boundary.

pub(super) fn io(error: nix::errno::Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(error as i32)
}
