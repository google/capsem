//! Darwin signals bind to the native PID version in the kernel. Exit uses
//! immutable process identity and status, never signal denial or exec version.
//! ABI/semantics: Apple XNU bsd/sys/proc_info_private.h and bsd/kern/proc_info.c.
use super::{ProcessId, Signal, SignalOutcome};
use nix::errno::Errno;
use std::{
    io,
    mem::{size_of, MaybeUninit},
    path::PathBuf,
    sync::OnceLock,
    time::{Duration, Instant},
};

const BSD_WITH_UNIQUE_ID: i32 = 18;

#[repr(C)]
struct UniqueInfo {
    executable_uuid: [u8; 16],
    unique_id: u64,
    parent_unique_id: u64,
    pid_version: i32,
    original_parent_version: i32,
    reserved: [u64; 2],
}

#[repr(C)]
struct ProcessInfo {
    bsd: libc::proc_bsdinfo,
    unique: UniqueInfo,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct AuditToken([u32; 8]);

// These layouts are fixed API structures asserted by Apple's headers.
const _: () =
    assert!(size_of::<UniqueInfo>() == 56 && size_of::<ProcessInfo>() == 192 && size_of::<AuditToken>() == 32);

type GuardedSignal = unsafe extern "C" fn(*mut AuditToken, libc::c_int) -> libc::c_int;

fn guarded_signal() -> io::Result<GuardedSignal> {
    static SIGNAL: OnceLock<Option<GuardedSignal>> = OnceLock::new();
    SIGNAL
        .get_or_init(|| {
            // macOS 14 has native identity queries but lacks this newer libproc
            // symbol. Resolve it without making the binary unloadable on 14.
            // SAFETY: RTLD_DEFAULT searches loaded system libraries; the static
            // symbol name is NUL-terminated and no pointer is retained by dlsym.
            let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"proc_signal_with_audittoken".as_ptr()) };
            if symbol.is_null() {
                None
            } else {
                // SAFETY: Apple libproc declares this exact fixed ABI signature.
                Some(unsafe { std::mem::transmute::<*mut libc::c_void, GuardedSignal>(symbol) })
            }
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "native PID-version guarded signals unavailable",
            )
        })
}

/// Identity of one original process, independent of numeric PID recycling.
pub struct ProcessHandle {
    pid: ProcessId,
    unique_id: u64,
    token: AuditToken,
}

impl ProcessHandle {
    pub fn pid(&self) -> ProcessId {
        self.pid
    }

    /// Resolve the executable for this native PID version, refusing a reused
    /// numeric PID before the caller can use the path as authorization.
    pub fn executable_path(&self) -> io::Result<PathBuf> {
        use std::os::unix::ffi::OsStringExt;
        if self.has_exited()? {
            return Err(io::Error::new(io::ErrorKind::NotFound, "process exited"));
        }
        let mut bytes = [0u8; 4096];
        // libproc has no nix wrapper. The buffer is writable and bounded;
        // proc_pidpath writes a NUL-terminated executable path.
        let size = unsafe { libc::proc_pidpath(self.pid.get() as i32, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
        if size <= 0 {
            return Err(io::Error::last_os_error());
        }
        if self.has_exited()? {
            return Err(io::Error::new(io::ErrorKind::NotFound, "process exited"));
        }
        let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(size as usize);
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes[..end].to_vec())))
    }
    pub fn open(pid: ProcessId) -> io::Result<Self> {
        let info = read_info(pid)?;
        let mut token = AuditToken([0; 8]);
        // XNU proc_find_audit_token uses PID and PID version, then captures and
        // revalidates immutable identity before applying ordinary cansignal.
        token.0[5] = pid.get();
        token.0[7] = info.unique.pid_version as u32;
        Ok(Self {
            pid,
            unique_id: info.unique.unique_id,
            token,
        })
    }

    pub fn signal(&self, signal: Signal) -> io::Result<SignalOutcome> {
        if self.has_exited()? {
            return Ok(SignalOutcome::Gone);
        }
        let send = guarded_signal()?;
        loop {
            let mut token = self.token;
            // SAFETY: the fixed 32-byte token is local and valid for the call;
            // its PID/version came from one native process record. The kernel
            // verifies that version before obtaining the referenced process.
            let code = unsafe { send(&mut token, signal.as_nix() as libc::c_int) };
            match code {
                0 => return Ok(SignalOutcome::Delivered),
                code if code == Errno::EINTR as i32 => continue,
                code if code == Errno::ESRCH as i32 && self.has_exited()? => return Ok(SignalOutcome::Gone),
                code => return Err(io::Error::from_raw_os_error(code)),
            }
        }
    }

    /// A changed exec/PID version is not exit. Only a missing original unique
    /// identity or its native zombie status proves that the process has gone.
    pub fn has_exited(&self) -> io::Result<bool> {
        match read_info(self.pid) {
            Ok(info) => Ok(info.unique.unique_id != self.unique_id || info.bsd.pbi_status == libc::SZOMB),
            Err(error) if error.raw_os_error() == Some(Errno::ESRCH as i32) => Ok(true),
            Err(error) => Err(error),
        }
    }

    /// Blocking deadline-bound native observation. Permission/read failures
    /// remain errors and never manufacture an exit receipt.
    pub fn wait_until_exit(&self, deadline: Instant) -> io::Result<bool> {
        loop {
            if self.has_exited()? {
                return Ok(true);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }
            std::thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }
}

fn read_info(pid: ProcessId) -> io::Result<ProcessInfo> {
    loop {
        let mut info = MaybeUninit::<ProcessInfo>::zeroed();
        // nix has no combined native status/immutable-ID query on Darwin.
        // SAFETY: the buffer has the exact documented ABI size, includes only
        // integers/arrays, and arg=1 includes unreaped zombie processes.
        let count = unsafe {
            libc::proc_pidinfo(
                pid.get() as i32,
                BSD_WITH_UNIQUE_ID,
                1,
                info.as_mut_ptr().cast(),
                size_of::<ProcessInfo>() as i32,
            )
        };
        if count == size_of::<ProcessInfo>() as i32 {
            let info = unsafe { info.assume_init() };
            if info.bsd.pbi_pid != pid.get() || info.unique.unique_id == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid native process identity",
                ));
            }
            return Ok(info);
        }
        if count != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete native process record",
            ));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(test)]
mod tests;
