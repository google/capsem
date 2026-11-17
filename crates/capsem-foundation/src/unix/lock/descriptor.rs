//! Advisory leases on file capabilities whose path may no longer exist.
use super::{errno, LockMode};
use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};
use std::fs::File;
use std::io;
use std::ops::{Deref, DerefMut};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};

#[derive(Debug)]
pub enum DescriptorLockAttempt {
    Acquired(DescriptorLock),
    /// The unchanged descriptor remains owned by the caller for later retry.
    Contended(File),
}

/// A kernel advisory lock released on drop. No path is opened or repaired.
#[derive(Debug)]
pub struct DescriptorLock(Flock<File>);
impl Deref for DescriptorLock {
    type Target = File;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for DescriptorLock {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl AsRawFd for DescriptorLock {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}
impl AsFd for DescriptorLock {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

/// Try a shared or exclusive lease on an already-authorized descriptor.
///
/// Use independently opened descriptions for competing leases: `try_clone`
/// aliases share the kernel lock ownership of the original description.
/// Expected contention returns the File without waiting. Other IO errors
/// close it. This does not add lockfile path or permission authority.
pub fn try_lock_descriptor(mut file: File, mode: LockMode) -> io::Result<DescriptorLockAttempt> {
    let argument = match mode {
        LockMode::Shared => FlockArg::LockSharedNonblock,
        LockMode::Exclusive => FlockArg::LockExclusiveNonblock,
    };
    loop {
        match Flock::lock(file, argument) {
            Ok(held) => return Ok(DescriptorLockAttempt::Acquired(DescriptorLock(held))),
            Err((returned, Errno::EINTR)) => file = returned,
            Err((returned, Errno::EWOULDBLOCK)) => return Ok(DescriptorLockAttempt::Contended(returned)),
            Err((_, error)) => return Err(errno::io(error)),
        }
    }
}

#[cfg(test)]
mod tests;
