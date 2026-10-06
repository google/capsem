//! Sticky change detection bound to already-open filesystem inodes.

#[cfg(target_os = "macos")]
use std::os::fd::AsFd;
use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd, OwnedFd},
};

#[cfg(target_os = "linux")]
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
#[cfg(target_os = "macos")]
use nix::sys::{
    event::{EventFilter, EventFlag, FilterFlag, KEvent, Kqueue},
    time::TimeSpec,
};

/// A nonblocking, conservative watch. Once changed (including overflow or
/// watch removal), it stays invalid. Construct a new watch for new proof.
/// Open descriptors remain owned for the entire watch lifetime.
pub struct ChangeWatch {
    #[cfg(target_os = "linux")]
    watcher: Inotify,
    #[cfg(target_os = "macos")]
    watcher: Kqueue,
    held: Vec<OwnedFd>,
    changed: bool,
}

impl ChangeWatch {
    pub fn new() -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        let watcher = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)?;
        #[cfg(target_os = "macos")]
        let watcher = {
            use nix::fcntl::{fcntl, FcntlArg, FdFlag};
            let queue = Kqueue::new()?;
            fcntl(queue.as_fd().as_raw_fd(), FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
            queue
        };
        Ok(Self {
            watcher,
            held: Vec::new(),
            changed: false,
        })
    }

    /// Add an already-contained open file or directory, never a pathname.
    pub fn add(&mut self, fd: BorrowedFd<'_>) -> io::Result<()> {
        let result = self.add_inner(fd);
        if result.is_err() {
            self.changed = true;
        }
        result
    }

    fn add_inner(&mut self, fd: BorrowedFd<'_>) -> io::Result<()> {
        let held = super::fd::duplicate(fd)?;
        #[cfg(target_os = "linux")]
        {
            // Linux has no fd form of inotify_add_watch. This process-owned
            // proc descriptor link resolves the held inode, not its old path.
            let path = format!("/proc/self/fd/{}", held.as_raw_fd());
            self.watcher.add_watch(
                path.as_str(),
                AddWatchFlags::IN_MODIFY
                    | AddWatchFlags::IN_ATTRIB
                    | AddWatchFlags::IN_CREATE
                    | AddWatchFlags::IN_DELETE
                    | AddWatchFlags::IN_MOVED_FROM
                    | AddWatchFlags::IN_MOVED_TO
                    | AddWatchFlags::IN_DELETE_SELF
                    | AddWatchFlags::IN_MOVE_SELF,
            )?;
        }
        #[cfg(target_os = "macos")]
        {
            let event = KEvent::new(
                held.as_raw_fd() as usize,
                EventFilter::EVFILT_VNODE,
                EventFlag::EV_ADD | EventFlag::EV_CLEAR,
                FilterFlag::NOTE_WRITE
                    | FilterFlag::NOTE_EXTEND
                    | FilterFlag::NOTE_ATTRIB
                    | FilterFlag::NOTE_LINK
                    | FilterFlag::NOTE_RENAME
                    | FilterFlag::NOTE_DELETE
                    | FilterFlag::NOTE_REVOKE,
                0,
                0,
            );
            self.watcher
                .kevent(&[event], &mut [], Some(*TimeSpec::new(0, 0).as_ref()))?;
        }
        self.held.push(held);
        Ok(())
    }

    /// Read at most one kernel event batch; never waits or reads file bytes.
    pub fn changed(&mut self) -> io::Result<bool> {
        if self.changed {
            return Ok(true);
        }
        let result = self.read_change();
        // An unreadable notification stream can never retain readiness.
        self.changed = !matches!(result, Ok(false));
        result
    }

    #[cfg(target_os = "linux")]
    fn read_change(&self) -> io::Result<bool> {
        match self.watcher.read_events() {
            Ok(events) => Ok(!events.is_empty()),
            Err(nix::errno::Errno::EAGAIN) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    #[cfg(target_os = "macos")]
    fn read_change(&self) -> io::Result<bool> {
        let blank = KEvent::new(
            0,
            EventFilter::EVFILT_VNODE,
            EventFlag::empty(),
            FilterFlag::empty(),
            0,
            0,
        );
        let count = self
            .watcher
            .kevent(&[], &mut [blank], Some(*TimeSpec::new(0, 0).as_ref()))?;
        Ok(count != 0)
    }
}

#[cfg(test)]
mod tests;
