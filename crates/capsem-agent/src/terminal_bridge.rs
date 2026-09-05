//! The terminal bridge: master PTY <-> vsock terminal port, one host
//! connection at a time.

use std::os::unix::io::RawFd;

use nix::libc;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

use crate::shutdown::{HostShutdown, HOLD_FOR_REPORT};
use crate::vsock_io::write_all_fd;

/// The shell going away ends the bridge, except during a host shutdown,
/// where the shell's exit is the shutdown doing its job: hold the
/// connection until the writer has put `ShutdownComplete` on the wire (or
/// `HOLD_FOR_REPORT` passes), so the report is not torn down with it.
fn hold_for_shutdown_report(shutdown: &HostShutdown) {
    if shutdown.is_requested() && !shutdown.wait_reported(HOLD_FOR_REPORT) {
        eprintln!("[capsem-agent] shutdown report was not written before the bridge closed");
    }
}

pub(crate) fn bridge_loop(master_fd: RawFd, vsock_fd: RawFd, shutdown: &HostShutdown) {
    let mut buf = [0u8; 8192];

    // Spawn a dedicated thread for vsock -> Master PTY (stdin direction)
    // This prevents deadlocks when both master_fd and vsock_fd buffers are full.
    let master_fd_clone = master_fd;
    let vsock_fd_clone = vsock_fd;
    std::thread::scope(|scope| {
        let reader = scope.spawn(move || {
            let mut local_buf = [0u8; 8192];
            loop {
                let mut poll_fds = [PollFd::new(
                    unsafe { std::os::unix::io::BorrowedFd::borrow_raw(vsock_fd_clone) },
                    PollFlags::POLLIN,
                )];

                match poll(&mut poll_fds, PollTimeout::from(1000u16)) {
                    Ok(0) => continue,
                    Ok(_) => {}
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(_) => break,
                }

                if let Some(revents) = poll_fds[0].revents() {
                    if revents.contains(PollFlags::POLLIN) {
                        match nix::unistd::read(vsock_fd_clone, &mut local_buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                if write_all_fd(master_fd_clone, &local_buf[..n]).is_err() {
                                    break;
                                }
                            }
                            Err(nix::errno::Errno::EAGAIN) => {}
                            Err(_) => break,
                        }
                    }
                    if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL) {
                        break;
                    }
                }
            }
        });

        loop {
            // Poll vsock_fd too so a local shutdown (triggered by the heartbeat
            // detecting host death) wakes us up via POLLHUP. Otherwise we'd sit
            // in poll forever waiting for PTY activity that never comes.
            let mut poll_fds = [
                PollFd::new(
                    unsafe { std::os::unix::io::BorrowedFd::borrow_raw(master_fd) },
                    PollFlags::POLLIN,
                ),
                PollFd::new(
                    unsafe { std::os::unix::io::BorrowedFd::borrow_raw(vsock_fd) },
                    PollFlags::empty(),
                ),
            ];

            match poll(&mut poll_fds, PollTimeout::from(1000u16)) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    eprintln!("[capsem-agent] poll error: {e}");
                    break;
                }
            }

            if let Some(revents) = poll_fds[1].revents() {
                if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL) {
                    break;
                }
            }

            // Master PTY -> vsock (stdout direction)
            if let Some(revents) = poll_fds[0].revents() {
                if revents.contains(PollFlags::POLLIN) {
                    match nix::unistd::read(master_fd, &mut buf) {
                        Ok(0) => {
                            hold_for_shutdown_report(shutdown);
                            break;
                        }
                        Ok(n) => {
                            if write_all_fd(vsock_fd, &buf[..n]).is_err() {
                                break;
                            }
                        }
                        Err(nix::errno::Errno::EAGAIN) => {}
                        Err(_) => {
                            hold_for_shutdown_report(shutdown);
                            break;
                        }
                    }
                }
                if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR) {
                    // The usual way a shell's exit shows up: a hangup on the
                    // master before any read returns zero.
                    hold_for_shutdown_report(shutdown);
                    break;
                }
            }
        }

        unsafe {
            libc::shutdown(vsock_fd, libc::SHUT_RDWR);
        }
        if reader.join().is_err() {
            eprintln!("[capsem-agent] terminal reader thread panicked");
        }
    });
}
