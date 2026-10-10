//! Sticky change detection bound to already-open filesystem inodes.

use super::contained::ContainedDir;
use std::{
    ffi::OsStr,
    io,
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    os::unix::fs::MetadataExt,
    path::{Component, Path},
};

#[cfg(target_os = "linux")]
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
};

#[cfg(target_os = "linux")]
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};
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
    #[cfg(target_os = "linux")]
    bindings: HashMap<WatchDescriptor, Option<HashSet<OsString>>>,
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
            #[cfg(target_os = "linux")]
            bindings: HashMap::new(),
            held: Vec::new(),
            changed: false,
        })
    }

    /// Add an already-contained open file or directory, never a pathname.
    pub fn add(&mut self, fd: BorrowedFd<'_>) -> io::Result<()> {
        let result = self.add_inner(fd, None, false);
        if result.is_err() {
            self.changed = true;
        }
        result
    }

    /// Bind a private absolute host directory's inode and namespace. Literal
    /// aliases and canonical ancestors are both watched; path work happens
    /// only during registration, never in polling. The returned descriptor
    /// cannot be redirected by a later replacement.
    pub fn open_directory(&mut self, path: &Path) -> io::Result<ContainedDir> {
        let result = self.open_directory_inner(path);
        if result.is_err() {
            self.changed = true;
        }
        result
    }

    fn open_directory_inner(&mut self, path: &Path) -> io::Result<ContainedDir> {
        if !path.is_absolute()
            || !path
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "watched directory must be a plain absolute path",
            ));
        }
        let root = ContainedDir::open_root(path)?;
        self.add(root.as_fd())?;
        let canonical = std::fs::canonicalize(path)?;
        self.add_path_bindings(path)?;
        if canonical != path {
            self.add_path_bindings(&canonical)?;
        }
        let original = root.metadata()?;
        let current = ContainedDir::open_root(path)?.metadata()?;
        let resolved = ContainedDir::open_root(&canonical)?.metadata()?;
        if (original.dev(), original.ino()) != (current.dev(), current.ino())
            || (original.dev(), original.ino()) != (resolved.dev(), resolved.ino())
        {
            return Err(io::Error::other(
                "watched directory binding changed during registration",
            ));
        }
        Ok(root)
    }

    fn add_path_bindings(&mut self, path: &Path) -> io::Result<()> {
        for child in path.ancestors() {
            let (Some(parent), Some(name)) = (child.parent(), child.file_name()) else {
                continue;
            };
            // A literal alias is watched through its enclosing directory;
            // the resolved hierarchy is separately covered by canonical.
            if std::fs::symlink_metadata(parent)?.file_type().is_symlink() {
                continue;
            }
            #[cfg(target_os = "macos")]
            if !std::fs::symlink_metadata(child)?.file_type().is_symlink() {
                // kqueue vnode events carry no child name. Watching the
                // enclosing directory would therefore turn every sibling
                // write into a false invalidation. The ancestor inode itself
                // reports rename/delete/revoke when its binding changes.
                let directory = ContainedDir::open_root(child)?;
                self.add_inner(directory.as_fd(), None, true)?;
                continue;
            }
            let directory = ContainedDir::open_root(parent)?;
            self.add_inner(directory.as_fd(), Some(name), false)?;
        }
        Ok(())
    }

    fn add_inner(&mut self, fd: BorrowedFd<'_>, name: Option<&OsStr>, binding_only: bool) -> io::Result<()> {
        let held = super::fd::duplicate(fd)?;
        #[cfg(target_os = "linux")]
        {
            let _ = binding_only;
            // Linux has no fd form of inotify_add_watch. This process-owned
            // proc descriptor link resolves the held inode, not its old path.
            let path = format!("/proc/self/fd/{}", held.as_raw_fd());
            let descriptor = self.watcher.add_watch(
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
            let binding = self.bindings.entry(descriptor).or_insert_with(|| Some(HashSet::new()));
            match name {
                None => *binding = None,
                Some(name) => {
                    if let Some(names) = binding {
                        names.insert(name.to_owned());
                    }
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            // Vnode events have no child name. A path binding therefore
            // watches only changes to its already-open ancestor inode;
            // NOTE_WRITE/EXTEND/LINK would report unrelated sibling activity.
            // Content watches retain every mutation flag.
            let _ = name;
            let filters = if binding_only {
                FilterFlag::NOTE_ATTRIB | FilterFlag::NOTE_RENAME | FilterFlag::NOTE_DELETE | FilterFlag::NOTE_REVOKE
            } else {
                FilterFlag::NOTE_WRITE
                    | FilterFlag::NOTE_EXTEND
                    | FilterFlag::NOTE_ATTRIB
                    | FilterFlag::NOTE_LINK
                    | FilterFlag::NOTE_RENAME
                    | FilterFlag::NOTE_DELETE
                    | FilterFlag::NOTE_REVOKE
            };
            let event = KEvent::new(
                held.as_raw_fd() as usize,
                EventFilter::EVFILT_VNODE,
                EventFlag::EV_ADD | EventFlag::EV_CLEAR,
                filters,
                0,
                0,
            );
            self.watcher
                .kevent(&[event], &mut [], Some(*TimeSpec::new(0, 0).as_ref()))?;
        }
        self.held.push(held);
        Ok(())
    }

    /// Read bounded nonblocking kernel batches; never traverses paths or
    /// reads file bytes. Excess ignored events conservatively invalidate.
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
        for _ in 0..2 {
            match self.watcher.read_events() {
                Ok(events) => {
                    if events.is_empty() {
                        return Ok(false);
                    }
                    for event in events {
                        match (self.bindings.get(&event.wd), event.name) {
                            (Some(Some(names)), Some(name)) if !names.contains(&name) => {}
                            _ => return Ok(true),
                        }
                    }
                }
                Err(nix::errno::Errno::EAGAIN) => return Ok(false),
                Err(error) => return Err(error.into()),
            }
        }
        // A relevant event may remain behind a large batch of siblings.
        Ok(true)
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
