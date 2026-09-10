//! Seccomp applies to every existing runtime thread; no open, connect, spawn,
//! signal, namespace, or filesystem mutation syscall is available afterwards.
use std::io;

pub(super) fn confine(_: u16) -> io::Result<()> {
    #[cfg(target_arch = "aarch64")]
    let architecture = 0xc00000b7;
    #[cfg(target_arch = "x86_64")]
    let architecture = 0xc000003e;
    let instruction = |code, jt, jf, k| libc::sock_filter { code, jt, jf, k };
    let mut filter = vec![
        instruction(0x20, 0, 0, 4),            // LD W ABS seccomp_data.arch
        instruction(0x15, 1, 0, architecture), // JEQ expected arch
        instruction(0x06, 0, 0, 0x80000000),   // RET KILL_PROCESS
        instruction(0x20, 0, 0, 0),            // LD W ABS seccomp_data.nr
    ];
    let allowed = [
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_close,
        libc::SYS_fcntl,
        libc::SYS_fstat,
        libc::SYS_recvfrom,
        libc::SYS_recvmsg,
        libc::SYS_sendto,
        libc::SYS_sendmsg,
        libc::SYS_accept4,
        libc::SYS_shutdown,
        libc::SYS_getsockopt,
        libc::SYS_setsockopt,
        libc::SYS_getsockname,
        libc::SYS_getpeername,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_pwait,
        libc::SYS_eventfd2,
        libc::SYS_futex,
        libc::SYS_mmap,
        libc::SYS_munmap,
        libc::SYS_mprotect,
        libc::SYS_madvise,
        libc::SYS_brk,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sigaltstack,
        libc::SYS_clock_gettime,
        libc::SYS_clock_nanosleep,
        libc::SYS_nanosleep,
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_getppid,
        libc::SYS_sched_yield,
        libc::SYS_sched_getaffinity,
        libc::SYS_getrandom,
        libc::SYS_exit,
        libc::SYS_exit_group,
    ];
    for syscall in allowed {
        filter.push(instruction(0x15, 0, 1, syscall as u32));
        filter.push(instruction(0x06, 0, 0, 0x7fff0000)); // RET ALLOW
    }
    #[cfg(target_arch = "x86_64")]
    for syscall in [libc::SYS_accept, libc::SYS_epoll_wait] {
        filter.push(instruction(0x15, 0, 1, syscall as u32));
        filter.push(instruction(0x06, 0, 0, 0x7fff0000));
    }
    filter.push(instruction(0x06, 0, 0, 0x00050000 | libc::EPERM as u32));
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_mut_ptr(),
    };
    // SAFETY: the kernel copies the initialized filter synchronously. TSYNC
    // refuses installation if any existing thread cannot receive this policy.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { libc::syscall(libc::SYS_seccomp, 1u32, 1u32, &program) };
    if result != 0 {
        return Err(if result < 0 {
            io::Error::last_os_error()
        } else {
            io::Error::other(format!("router seccomp could not confine thread {result}"))
        });
    }
    Ok(())
}
