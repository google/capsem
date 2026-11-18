//! Prevent a same-user script reopening PID1's private status FD through procfs.
use std::io;

pub(super) struct Guard(i32);
impl Guard {
    pub(super) fn install() -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let previous = unsafe { nix::libc::prctl(nix::libc::PR_GET_DUMPABLE) };
            if previous < 0 || unsafe { nix::libc::prctl(nix::libc::PR_SET_DUMPABLE, 0) } < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(previous))
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(Self(0))
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        unsafe {
            let _ = nix::libc::prctl(nix::libc::PR_SET_DUMPABLE, self.0);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = self.0;
    }
}
