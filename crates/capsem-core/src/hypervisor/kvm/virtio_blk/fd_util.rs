//! Raw eventfd, epoll, and interrupt-signalling helpers for the block worker.

use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicU32, Ordering};

pub(super) fn dup_owned_fd(fd: RawFd) -> std::io::Result<OwnedFd> {
    let duped = unsafe { libc::dup(fd) };
    if duped < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(duped) })
}

pub(super) fn create_eventfd(flags: libc::c_int) -> std::io::Result<OwnedFd> {
    let fd = unsafe { libc::eventfd(0, flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

pub(super) fn create_epoll_fd() -> std::io::Result<OwnedFd> {
    let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

pub(super) fn epoll_add(epoll_fd: RawFd, fd: RawFd, token: u64) -> std::io::Result<()> {
    let mut event = libc::epoll_event {
        events: libc::EPOLLIN as u32,
        u64: token,
    };
    let ret = unsafe { libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut event) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn epoll_wait_tokens(epoll_fd: RawFd) -> std::io::Result<Vec<u64>> {
    let mut events = [libc::epoll_event { events: 0, u64: 0 }; 8];
    loop {
        let n = unsafe { libc::epoll_wait(epoll_fd, events.as_mut_ptr(), events.len() as i32, -1) };
        if n >= 0 {
            return Ok(events[..n as usize].iter().map(|event| event.u64).collect());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return Err(error);
    }
}

pub(super) fn read_eventfd(fd: RawFd) -> std::io::Result<u64> {
    let mut val = 0_u64;
    loop {
        let ret = unsafe {
            libc::read(
                fd,
                &mut val as *mut u64 as *mut libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if ret == std::mem::size_of::<u64>() as isize {
            return Ok(val);
        }
        if ret < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "short eventfd read",
        ));
    }
}

pub(super) fn drain_eventfd(fd: RawFd) -> std::io::Result<Option<u64>> {
    match read_eventfd(fd) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.raw_os_error() == Some(libc::EAGAIN) => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn write_eventfd(fd: RawFd) -> std::io::Result<()> {
    let val = 1_u64;
    loop {
        let ret = unsafe {
            libc::write(
                fd,
                &val as *const u64 as *const libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if ret == std::mem::size_of::<u64>() as isize {
            return Ok(());
        }
        if ret < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "short eventfd write",
        ));
    }
}

pub(super) fn signal_irq(irq_fd: RawFd, interrupt_status: &AtomicU32) {
    interrupt_status.fetch_or(1, Ordering::SeqCst);
    let val: u64 = 1;
    let ret = unsafe { libc::write(irq_fd, &val as *const u64 as *const libc::c_void, 8) };
    if ret < 0 {
        tracing::warn!(
            event_name = "virtio.blk.irq_signal_failed",
            error = %std::io::Error::last_os_error(),
            "failed to signal virtio-blk interrupt eventfd"
        );
    }
}
