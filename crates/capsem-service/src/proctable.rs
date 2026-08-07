//! The process table, read with a syscall instead of by spawning `ps`.
//!
//! `/bin/ps` is setuid root, and macOS refuses to `execvp` a setuid binary
//! from a sandboxed process whatever the profile permits -- `(allow default)`
//! still gives `Operation not permitted`. The service's orphan reaper shelled
//! out to it and treated a failed spawn as "no orphans", so under the release
//! gate's Seatbelt profile a restarting service reaped nothing and its per-VM
//! children outlived the whole run.
//!
//! The output is deliberately `ps -ax -o pid=,command=` shaped -- `"<pid>
//! <argv joined by spaces>"` -- because the matching that consumes it is
//! tested against that shape and is the part that was never wrong.
//!
//! Matching argv rather than remembering pids is also what makes the reaper
//! safe against pid reuse: a recorded pid can name an unrelated process by
//! the time a crashed service restarts, whereas a command line naming this
//! run directory cannot.

use std::io;

/// Every running process, one `"<pid> <argv>"` line each.
///
/// Processes that cannot be read are skipped rather than failing the sweep:
/// on macOS `KERN_PROCARGS2` is denied for processes owned by other users,
/// which is most of the system and none of ours.
pub fn running_processes() -> io::Result<String> {
    imp::running_processes()
}

#[cfg(target_os = "macos")]
mod imp {
    use nix::libc;
    use std::io;

    // SAFETY: `libproc` is part of macOS and this is its documented
    // signature. Used instead of walking a `KERN_PROC_ALL` buffer because
    // that requires `kinfo_proc`, whose layout this `libc` does not expose --
    // and `capsem.gate.pidfiles` already reaches for libproc through ctypes
    // for the same reason, so the two agree on one primitive.
    unsafe extern "C" {
        fn proc_listallpids(buffer: *mut libc::c_void, buffersize: libc::c_int) -> libc::c_int;
    }

    fn sysctl_bytes(name: &[libc::c_int]) -> io::Result<Vec<u8>> {
        let mut length: libc::size_t = 0;
        // Sized first, then read. The table changes between the two calls, so
        // the second is given the size the first reported and whatever it
        // actually writes is what gets parsed -- a process that appeared in
        // between is simply not in this sweep.
        let sized = unsafe {
            libc::sysctl(
                name.as_ptr().cast_mut(),
                name.len() as libc::c_uint,
                std::ptr::null_mut(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if sized != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u8; length];
        let read = unsafe {
            libc::sysctl(
                name.as_ptr().cast_mut(),
                name.len() as libc::c_uint,
                buffer.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 {
            return Err(io::Error::last_os_error());
        }
        buffer.truncate(length);
        Ok(buffer)
    }

    fn pids() -> io::Result<Vec<libc::pid_t>> {
        // Sized first with a null buffer, which reports the bytes needed.
        let sized = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
        if sized <= 0 {
            return Err(io::Error::last_os_error());
        }
        // Room to spare: processes start between the sizing call and the
        // filling one, and a buffer that is exactly the reported size
        // silently truncates when they do.
        let capacity = sized as usize + 64;
        let mut buffer = vec![0i32; capacity];
        let bytes = (capacity * std::mem::size_of::<i32>()) as libc::c_int;
        let written = unsafe { proc_listallpids(buffer.as_mut_ptr().cast(), bytes) };
        if written <= 0 {
            return Err(io::Error::last_os_error());
        }
        buffer.truncate(written as usize);
        Ok(buffer.into_iter().filter(|pid| *pid > 0).collect())
    }

    /// The full argument vector of one process, joined by spaces.
    ///
    /// `KERN_PROCARGS2` returns `argc`, then the executable path, then NUL
    /// padding, then `argc` NUL-terminated arguments, then the environment.
    /// Only the arguments are wanted: the environment carries secrets, and
    /// this string is matched against and logged.
    fn argv(pid: libc::pid_t) -> Option<String> {
        let name = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        let buffer = sysctl_bytes(&name).ok()?;
        let count = u32::from_ne_bytes(buffer.get(..4)?.try_into().ok()?) as usize;

        let mut rest = buffer.get(4..)?;
        // The executable path, then the padding NULs separating it from argv.
        let path_end = rest.iter().position(|byte| *byte == 0)?;
        rest = rest.get(path_end..)?;
        let start = rest.iter().position(|byte| *byte != 0)?;
        rest = rest.get(start..)?;

        let mut arguments = Vec::with_capacity(count);
        for field in rest.split(|byte| *byte == 0).take(count) {
            arguments.push(String::from_utf8_lossy(field).into_owned());
        }
        Some(arguments.join(" "))
    }

    pub(super) fn running_processes() -> io::Result<String> {
        let mut lines = String::new();
        for pid in pids()? {
            // Unreadable is ordinary: `KERN_PROCARGS2` is denied for other
            // users' processes, and ours are not among those.
            if let Some(arguments) = argv(pid) {
                lines.push_str(&format!("{pid} {arguments}\n"));
            }
        }
        Ok(lines)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::io;

    pub(super) fn running_processes() -> io::Result<String> {
        let mut lines = String::new();
        for entry in std::fs::read_dir("/proc")?.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            // A process that exits mid-scan takes its `/proc` entry with it.
            let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
                continue;
            };
            let arguments: Vec<String> = raw
                .split(|byte| *byte == 0)
                .filter(|field| !field.is_empty())
                .map(|field| String::from_utf8_lossy(field).into_owned())
                .collect();
            if arguments.is_empty() {
                continue;
            }
            lines.push_str(&format!("{pid} {}\n", arguments.join(" ")));
        }
        Ok(lines)
    }
}

#[cfg(test)]
mod tests;
