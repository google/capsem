use super::{Access, Policy, Role};
use nix::fcntl::{open, OFlag};
use nix::sys::stat::{fstat, Mode, SFlag};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

const CREATE_RULESET_VERSION: u32 = 1;
const RESTRICT_SELF_TSYNC: u32 = 1 << 3;
const RULE_PATH_BENEATH: u32 = 1;
const MINIMUM_ABI: i32 = 6;

const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_REMOVE_DIR: u64 = 1 << 4;
const FS_REMOVE_FILE: u64 = 1 << 5;
const FS_MAKE_DIR: u64 = 1 << 7;
const FS_MAKE_REG: u64 = 1 << 8;
const FS_MAKE_SOCK: u64 = 1 << 9;
const FS_MAKE_FIFO: u64 = 1 << 10;
const FS_MAKE_SYM: u64 = 1 << 12;
const FS_REFER: u64 = 1 << 13;
const FS_TRUNCATE: u64 = 1 << 14;
const FS_IOCTL_DEV: u64 = 1 << 15;
const FS_HANDLED: u64 = FS_EXECUTE
    | FS_WRITE_FILE
    | FS_READ_FILE
    | FS_READ_DIR
    | FS_REMOVE_DIR
    | FS_REMOVE_FILE
    | FS_MAKE_DIR
    | FS_MAKE_REG
    | FS_MAKE_SOCK
    | FS_MAKE_FIFO
    | FS_MAKE_SYM
    | FS_REFER
    | FS_TRUNCATE
    | FS_IOCTL_DEV;
const FS_READ: u64 = FS_READ_FILE | FS_READ_DIR;
const FS_WRITE: u64 = FS_READ
    | FS_WRITE_FILE
    | FS_REMOVE_DIR
    | FS_REMOVE_FILE
    | FS_MAKE_DIR
    | FS_MAKE_REG
    | FS_MAKE_SOCK
    | FS_MAKE_FIFO
    | FS_MAKE_SYM
    | FS_REFER
    | FS_TRUNCATE;

const NET_BIND_TCP: u64 = 1 << 0;
const NET_CONNECT_TCP: u64 = 1 << 1;
const NET_HANDLED: u64 = NET_BIND_TCP | NET_CONNECT_TCP;
const SCOPE_ABSTRACT_UNIX_SOCKET: u64 = 1 << 0;
const SCOPE_SIGNAL: u64 = 1 << 1;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

pub(super) fn confine(policy: &Policy) -> io::Result<()> {
    let ruleset = create_ruleset()?;
    for rule in policy.paths() {
        add_path_rule(&ruleset, rule.path(), rule.access())?;
    }
    nix::sys::prctl::set_no_new_privs().map_err(super::super::errno::io)?;
    restrict_all_threads(&ruleset)?;
    install_seccomp(policy.role())
}

fn create_ruleset() -> io::Result<OwnedFd> {
    // nix has no Landlock API. The kernel reads no attribute when VERSION is
    // set, and returns the supported ABI directly.
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    };
    if abi < 0 {
        return Err(io::Error::last_os_error());
    }
    if abi < i64::from(MINIMUM_ABI) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("Landlock ABI {MINIMUM_ABI} required; kernel provides {abi}"),
        ));
    }
    let attr = RulesetAttr {
        handled_access_fs: FS_HANDLED,
        handled_access_net: NET_HANDLED,
        scoped: SCOPE_ABSTRACT_UNIX_SOCKET | SCOPE_SIGNAL,
    };
    // nix has no Landlock API. The initialized C-layout attribute is copied
    // synchronously by the kernel.
    let descriptor = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attr,
            std::mem::size_of::<RulesetAttr>(),
            0u32,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful create_ruleset result is a new owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor as i32) })
}

fn add_path_rule(ruleset: &OwnedFd, path: &std::path::Path, access: Access) -> io::Result<()> {
    let raw = open(
        path,
        OFlag::O_PATH | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )
    .map_err(super::super::errno::io)?;
    // SAFETY: nix::fcntl::open returned a new owned descriptor.
    let target = unsafe { OwnedFd::from_raw_fd(raw) };
    let metadata = fstat(target.as_raw_fd()).map_err(super::super::errno::io)?;
    let kind = SFlag::from_bits_truncate(metadata.st_mode);
    if kind.contains(SFlag::S_IFLNK) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("sandbox grant is a symlink: {}", path.display()),
        ));
    }
    let directory = kind.contains(SFlag::S_IFDIR);
    let allowed_access = match (access, directory) {
        (Access::ReadOnly, true) => FS_READ,
        (Access::ReadOnly, false) => FS_READ_FILE,
        (Access::ReadWrite, true) => FS_WRITE,
        (Access::ReadWrite, false) => FS_READ_FILE | FS_WRITE_FILE | FS_TRUNCATE,
        (Access::Executable, false) => FS_READ_FILE | FS_EXECUTE,
        (Access::Executable, true) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("sandbox executable grant is a directory: {}", path.display()),
            ));
        }
    };
    let attr = PathBeneathAttr {
        allowed_access,
        parent_fd: target.as_raw_fd(),
    };
    // nix has no Landlock API. Both descriptors remain live while the kernel
    // synchronously copies the packed rule.
    let result = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset.as_raw_fd(),
            RULE_PATH_BENEATH,
            &attr,
            0u32,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn restrict_all_threads(ruleset: &OwnedFd) -> io::Result<()> {
    // nix has no Landlock API. TSYNC is mandatory: workers create their Tokio
    // runtime before confinement, and a single ambient sibling thread would
    // invalidate the process boundary.
    let result = unsafe {
        libc::syscall(
            libc::SYS_landlock_restrict_self,
            ruleset.as_raw_fd(),
            RESTRICT_SELF_TSYNC,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn install_seccomp(role: Role) -> io::Result<()> {
    #[cfg(target_arch = "aarch64")]
    let architecture = 0xc00000b7;
    #[cfg(target_arch = "x86_64")]
    let architecture = 0xc000003e;
    let instruction = |code, jt, jf, k| libc::sock_filter { code, jt, jf, k };
    let deny = 0x0005_0000 | libc::EPERM as u32;
    let allow = 0x7fff_0000;
    let mut filter = vec![
        instruction(0x20, 0, 0, 4),            // LD W ABS seccomp_data.arch
        instruction(0x15, 1, 0, architecture), // JEQ expected arch
        instruction(0x06, 0, 0, 0x8000_0000),  // RET KILL_PROCESS
        instruction(0x20, 0, 0, 0),            // LD W ABS seccomp_data.nr
    ];
    let mut denied = vec![
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_kcmp,
        libc::SYS_pidfd_getfd,
        libc::SYS_open_by_handle_at,
        libc::SYS_name_to_handle_at,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_setns,
        libc::SYS_unshare,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_keyctl,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_userfaultfd,
        libc::SYS_memfd_create,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_register,
        libc::SYS_reboot,
        libc::SYS_kexec_load,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_chmod,
        libc::SYS_fchmod,
        libc::SYS_fchmodat,
        libc::SYS_chown,
        libc::SYS_fchown,
        libc::SYS_lchown,
        libc::SYS_fchownat,
        libc::SYS_setxattr,
        libc::SYS_lsetxattr,
        libc::SYS_fsetxattr,
        libc::SYS_removexattr,
        libc::SYS_lremovexattr,
        libc::SYS_fremovexattr,
        libc::SYS_utime,
        libc::SYS_utimes,
        libc::SYS_futimesat,
        libc::SYS_utimensat,
    ];
    if role == Role::Gateway {
        denied.extend([libc::SYS_execve, libc::SYS_execveat]);
    }
    for syscall in denied {
        filter.push(instruction(0x15, 0, 1, syscall as u32));
        filter.push(instruction(0x06, 0, 0, deny));
    }
    // New AF_UNIX sockets support descriptor channels and child router
    // socketpairs. Internet, packet, netlink and VSOCK sockets must arrive as
    // already-connected grants from the trusted coordinator.
    filter.extend([
        instruction(0x15, 1, 0, libc::SYS_socket as u32),
        instruction(0x06, 0, 0, allow),
        instruction(0x20, 0, 0, 16), // LD W ABS seccomp_data.args[0]
        instruction(0x15, 0, 1, libc::AF_UNIX as u32),
        instruction(0x06, 0, 0, allow),
        instruction(0x06, 0, 0, deny),
    ]);
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_mut_ptr(),
    };
    // SAFETY: the kernel copies the initialized filter synchronously. TSYNC
    // refuses installation if any existing thread cannot receive the policy.
    let result = unsafe { libc::syscall(libc::SYS_seccomp, 1u32, 1u32, &program) };
    if result != 0 {
        return Err(if result < 0 {
            io::Error::last_os_error()
        } else {
            io::Error::other(format!("worker seccomp could not confine thread {result}"))
        });
    }
    Ok(())
}

use std::os::fd::FromRawFd;
