//! Kernel execution context for durable numeric PID witnesses.
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessScope {
    boot: String,
    namespace: Option<[u64; 2]>,
}

impl ProcessScope {
    pub fn current() -> io::Result<Self> {
        let scope = current()?;
        scope.validate()?;
        Ok(scope)
    }

    pub fn validate(&self) -> io::Result<()> {
        if !valid_boot_id(&self.boot) || self.namespace.is_some_and(|id| id[1] == 0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid kernel process scope",
            ));
        }
        Ok(())
    }
}

fn valid_boot_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
        && value.bytes().any(|b| b.is_ascii_hexdigit() && b != b'0')
}

#[cfg(target_os = "linux")]
fn current() -> io::Result<ProcessScope> {
    use std::{io::Read, os::unix::fs::MetadataExt};
    let mut bytes = Vec::new();
    std::fs::File::open("/proc/sys/kernel/random/boot_id")?
        .take(65)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "kernel boot identity exceeds limit",
        ));
    }
    let boot = std::str::from_utf8(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid kernel boot identity"))?
        .trim()
        .to_ascii_lowercase();
    // This is a kernel-owned namespace link, independent of user/guest paths.
    // Retain the open descriptor while reading the namespace's own inode.
    let namespace = std::fs::File::open("/proc/self/ns/pid")?.metadata()?;
    Ok(ProcessScope {
        boot,
        namespace: Some([namespace.dev(), namespace.ino()]),
    })
}

#[cfg(target_os = "macos")]
fn current() -> io::Result<ProcessScope> {
    let mut bytes = [0u8; 64];
    let mut size = bytes.len();
    // nix has no Darwin named sysctl wrapper. Apple XNU kern_sysctl.c exports
    // kern.bootsessionuuid as the boot session's fixed UUID string.
    // SAFETY: a NUL-terminated name and bounded writable buffer are provided;
    // no new value is supplied, so this operation cannot mutate the kernel.
    let result = unsafe {
        libc::sysctlbyname(
            c"kern.bootsessionuuid".as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if size > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "kernel boot identity exceeds limit",
        ));
    }
    let end = bytes[..size].iter().position(|b| *b == 0).unwrap_or(size);
    let boot = std::str::from_utf8(&bytes[..end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid kernel boot identity"))?
        .trim()
        .to_ascii_lowercase();
    Ok(ProcessScope { boot, namespace: None })
}

#[cfg(test)]
mod tests;
