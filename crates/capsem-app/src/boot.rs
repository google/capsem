use std::mem::ManuallyDrop;
use std::os::unix::io::{FromRawFd, RawFd};

pub use capsem_core::{
    boot_vm,
    create_net_state,
    read_control_msg,
    write_control_msg,
    send_boot_config,
};

/// Clone a raw fd into an independently-owned File.
/// The original fd remains open and unaffected.
pub(crate) fn clone_fd(fd: RawFd) -> std::io::Result<std::fs::File> {
    // Safety: fd is valid (checked by caller context)
    let file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    file.try_clone() // creates a dup'd fd owned by the returned File
}
