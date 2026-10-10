use super::*;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::fd::AsFd;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn seatbelt_profile_keeps_grant_values_out_of_policy_source() {
    let policy = Policy::new(Role::VmOwner)
        .allow("/session/with \"quotes\"", Access::ReadWrite)
        .allow("/assets", Access::ReadOnly)
        .allow("/opt/capsem-router", Access::Executable);
    let compiled = super::seatbelt::compile(&policy).unwrap();
    let source = compiled.source().to_str().unwrap();
    assert!(!source.contains("/session"));
    assert!(!source.contains("/assets"));
    assert!(!source.contains("/opt/capsem-router"));
    assert!(source.contains("(deny file-read*)"));
    assert!(source.contains("(deny file-write*)"));
    assert!(source.contains("(deny network-outbound)"));
    assert!(source.contains("(deny network-bind)"));
    assert!(source.contains("(deny process-exec)"));
    assert!(source.contains("(deny signal)"));
    assert!(source.contains("(allow signal (target same-sandbox))"));
    assert!(source.contains("(deny file-write-mode file-write-owner file-write-setugid)"));
    assert!(source.contains("(allow process-exec (with no-sandbox) (literal (param \"PATH_2\")))"));
    assert_eq!(
        compiled.parameters(),
        [
            ("PATH_0", "/session/with \"quotes\""),
            ("PATH_1", "/assets"),
            ("PATH_2", "/opt/capsem-router"),
        ]
    );
}

#[test]
fn seatbelt_gateway_accepts_only_prebound_inbound_network() {
    let compiled = super::seatbelt::compile(&Policy::new(Role::Gateway)).unwrap();
    let source = compiled.source().to_str().unwrap();
    assert!(source.contains("(allow network-inbound (local ip \"*:*\"))"));
    assert!(!source.contains("allow network-outbound"));
    assert!(!source.contains("allow network-bind"));
}

#[test]
fn seatbelt_ledger_is_deny_by_default_with_only_its_directory() {
    let compiled =
        super::seatbelt::compile(&Policy::new(Role::Ledger).allow("/sessions/one", Access::ReadWrite)).unwrap();
    let source = compiled.source().to_str().unwrap();
    assert!(source.starts_with("(version 1)\n(deny default)\n"));
    assert!(!source.contains("allow network"));
    assert!(!source.contains("allow process-exec"));
    assert!(!source.contains("allow signal"));
    assert!(!source.contains("allow mach-lookup"));
    assert!(source.contains("(allow file-read* file-write*"));
    assert_eq!(compiled.parameters(), [("PATH_0", "/sessions/one")]);
}

#[test]
fn seatbelt_proxy_is_deny_by_default_and_descriptor_only() {
    let compiled = super::seatbelt::compile(&Policy::new(Role::Proxy)).unwrap();
    let source = compiled.source().to_str().unwrap();
    assert!(source.starts_with("(version 1)\n(deny default)\n"));
    assert!(!source.contains("allow file"));
    assert!(!source.contains("allow network"));
    assert!(!source.contains("allow process-exec"));
    assert!(!source.contains("allow signal"));
    assert!(!source.contains("allow mach-lookup"));
    assert!(compiled.parameters().is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn macos_policy_compiles_canonical_grant_paths() {
    let directory = tempfile::tempdir().unwrap();
    let policy = Policy::new(Role::Ledger).allow(directory.path(), Access::ReadWrite);
    let canonical = super::platform::canonical_policy(&policy).unwrap();
    assert_eq!(canonical.paths()[0].path(), directory.path().canonicalize().unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn old_landlock_abis_require_single_threaded_startup() {
    assert_eq!(super::platform::restriction_flags_for(6, 1).unwrap(), 0);
    let error = super::platform::restriction_flags_for(7, 2).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    assert!(error.to_string().contains("single-threaded worker startup"));
}

#[cfg(target_os = "linux")]
#[test]
fn landlock_abi_eight_synchronizes_existing_threads() {
    assert_ne!(super::platform::restriction_flags_for(8, usize::MAX).unwrap(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_device_grant_preserves_kvm_ioctls() {
    let kvm = std::path::Path::new("/dev/kvm");
    if !kvm.exists() {
        return;
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "unix::worker_sandbox::tests::linux_kvm_device_child",
            "--nocapture",
        ])
        .env_clear()
        .env("WORKER_SANDBOX_KVM", kvm)
        .status()
        .unwrap();
    if status.code() == Some(77) {
        // Coverage runtimes may start a profiler thread before the test body.
        // Landlock ABIs before TSYNC cannot confine that artificial sibling;
        // production workers are explicitly single-threaded at confinement.
        return;
    }
    assert!(status.success(), "sandboxed KVM probe failed: {status}");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_kvm_device_child() {
    let Some(kvm) = std::env::var_os("WORKER_SANDBOX_KVM") else {
        return;
    };
    let kvm = std::path::PathBuf::from(kvm);
    let device = std::fs::OpenOptions::new().read(true).write(true).open(&kvm).unwrap();
    if let Err(error) = confine(&Policy::new(Role::VmOwner).allow(&kvm, Access::ReadWriteDevice)) {
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("single-threaded worker startup"));
        std::process::exit(77);
    }

    // nix's no-argument ioctl wrapper omits the variadic argument while the
    // KVM ABI expects an explicit zero. The descriptor and constant are valid
    // for KVM_GET_API_VERSION, which reads no userspace pointer.
    let version = unsafe { libc::ioctl(device.as_raw_fd(), 0xae00, 0u64) };
    assert_eq!(
        version,
        12,
        "KVM_GET_API_VERSION failed: {}",
        std::io::Error::last_os_error()
    );
    std::process::exit(0);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_device_grant_rejects_regular_files() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let error = confine(&Policy::new(Role::VmOwner).allow(file.path(), Access::ReadWriteDevice)).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("sandbox device grant is not a device"));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_policy_preserves_grants_and_denies_ambient_authority() {
    let directory = tempfile::tempdir().unwrap();
    let allowed = directory.path().join("allowed");
    let readonly = directory.path().join("readonly");
    let denied = directory.path().join("denied");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&readonly).unwrap();
    std::fs::create_dir_all(&denied).unwrap();
    std::fs::write(readonly.join("value"), b"readable").unwrap();
    std::fs::write(denied.join("granted"), b"one file").unwrap();
    std::fs::write(denied.join("secret"), b"secret").unwrap();
    let control = denied.join("control.sock");
    let _control = UnixListener::bind(&control).unwrap();

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    let (socket, _) = listener.accept().unwrap();

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "unix::worker_sandbox::tests::linux_sandbox_child",
            "--nocapture",
        ])
        .env_clear()
        .env("WORKER_SANDBOX_ALLOWED", &allowed)
        .env("WORKER_SANDBOX_READONLY", &readonly)
        .env("WORKER_SANDBOX_DENIED", &denied)
        .env("WORKER_SANDBOX_EXACT", denied.join("granted"))
        .env("WORKER_SANDBOX_CONTROL", &control)
        .env("WORKER_SANDBOX_PORT", port.to_string())
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(socket)))
        .spawn()
        .unwrap();
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    client.write_all(b"ping").unwrap();
    let mut reply = [0; 4];
    let read = client.read_exact(&mut reply);
    let status = child.wait().unwrap();
    assert!(status.success(), "sandbox child failed: {status}");
    read.unwrap();
    assert!(matches!(&reply, b"pong" | b"safe"));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_sandbox_child() {
    let Some(allowed) = std::env::var_os("WORKER_SANDBOX_ALLOWED") else {
        return;
    };
    let readonly = std::path::PathBuf::from(std::env::var_os("WORKER_SANDBOX_READONLY").unwrap());
    let denied = std::path::PathBuf::from(std::env::var_os("WORKER_SANDBOX_DENIED").unwrap());
    let exact = std::path::PathBuf::from(std::env::var_os("WORKER_SANDBOX_EXACT").unwrap());
    let control = std::env::var_os("WORKER_SANDBOX_CONTROL").unwrap();
    let port: u16 = std::env::var("WORKER_SANDBOX_PORT").unwrap().parse().unwrap();
    let allowed = std::path::PathBuf::from(allowed);
    let stdin = std::io::stdin();
    let mut inherited = TcpStream::from(super::super::fd::duplicate(stdin.as_fd()).unwrap());
    // Tokio prepares its signal-driver socketpair while building a runtime.
    // A current-thread runtime owns no sibling threads before confinement;
    // its later blocking workers inherit the installed policy.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    if let Err(error) = confine(
        &Policy::new(Role::Ledger)
            .allow(&allowed, Access::ReadWrite)
            .allow(&readonly, Access::ReadOnly)
            .allow(&exact, Access::ReadOnly),
    ) {
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("single-threaded worker startup"));
        inherited.write_all(b"safe").unwrap();
        std::process::exit(0);
    }

    runtime.block_on(async {
        tokio::task::spawn_blocking(move || {
            assert_eq!(std::fs::read(readonly.join("value")).unwrap(), b"readable");
            assert_eq!(std::fs::read(exact).unwrap(), b"one file");
            assert!(std::fs::write(readonly.join("value"), b"changed").is_err());
            std::fs::write(allowed.join("created"), b"ok").unwrap();
            let atomic = allowed.join("atomic");
            super::super::fs::atomic_write_private(&atomic, b"private").unwrap();
            assert_eq!(std::fs::read(&atomic).unwrap(), b"private");
            assert_eq!(
                std::fs::symlink_metadata(&atomic).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert!(std::fs::read(denied.join("secret")).is_err());
            assert!(UnixStream::connect(control).is_err());
            assert!(UnixStream::pair().is_err());
            assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err());
            assert!(Command::new("/usr/bin/true").status().is_err());
            let parent = unsafe { libc::getppid() };
            assert_eq!(unsafe { libc::kill(parent, 0) }, -1, "worker signalled its parent");
        })
        .await
        .unwrap();
        tokio::task::spawn_blocking(|| 7usize).await.unwrap()
    });

    let mut request = [0; 4];
    inherited.read_exact(&mut request).unwrap();
    assert_eq!(&request, b"ping");
    inherited.write_all(b"pong").unwrap();
    std::process::exit(0);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_policy_preserves_grants_and_denies_ambient_authority() {
    let directory = tempfile::tempdir().unwrap();
    let allowed = directory.path().join("allowed");
    let readonly = directory.path().join("readonly");
    let denied = directory.path().join("denied");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&readonly).unwrap();
    std::fs::create_dir_all(&denied).unwrap();
    let allowed = allowed.canonicalize().unwrap();
    let readonly = readonly.canonicalize().unwrap();
    let denied = denied.canonicalize().unwrap();
    std::fs::write(readonly.join("value"), b"readable").unwrap();
    std::fs::write(denied.join("secret"), b"secret").unwrap();
    let control = denied.join("control.sock");
    let _control = UnixListener::bind(&control).unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    let (socket, _) = listener.accept().unwrap();

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "unix::worker_sandbox::tests::macos_sandbox_child",
            "--nocapture",
        ])
        .env_clear()
        .env("WORKER_SANDBOX_ALLOWED", &allowed)
        .env("WORKER_SANDBOX_READONLY", &readonly)
        .env("WORKER_SANDBOX_DENIED", &denied)
        .env("WORKER_SANDBOX_CONTROL", &control)
        .env("WORKER_SANDBOX_PORT", port.to_string())
        .env("LLVM_PROFILE_FILE", allowed.join("profile-%p-%m.profraw"))
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(socket)))
        .spawn()
        .unwrap();
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    client.write_all(b"ping").unwrap();
    let mut reply = [0; 4];
    let read = client.read_exact(&mut reply);
    let status = child.wait().unwrap();
    assert!(status.success(), "sandbox child failed: {status}");
    read.unwrap();
    assert_eq!(&reply, b"pong");
}

#[cfg(target_os = "macos")]
#[test]
fn macos_sandbox_child() {
    use std::os::unix::fs::PermissionsExt;

    let Some(allowed) = std::env::var_os("WORKER_SANDBOX_ALLOWED") else {
        return;
    };
    let allowed = std::path::PathBuf::from(allowed);
    let readonly = std::path::PathBuf::from(std::env::var_os("WORKER_SANDBOX_READONLY").unwrap());
    let denied = std::path::PathBuf::from(std::env::var_os("WORKER_SANDBOX_DENIED").unwrap());
    let control = std::env::var_os("WORKER_SANDBOX_CONTROL").unwrap();
    let port: u16 = std::env::var("WORKER_SANDBOX_PORT").unwrap().parse().unwrap();
    let stdin = std::io::stdin();
    let mut inherited = TcpStream::from(super::super::fd::duplicate(stdin.as_fd()).unwrap());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    confine(
        &Policy::new(Role::Ledger)
            .allow(&allowed, Access::ReadWrite)
            .allow(&readonly, Access::ReadOnly),
    )
    .unwrap();

    runtime.block_on(async {
        tokio::task::spawn_blocking(move || {
            assert_eq!(std::fs::read(readonly.join("value")).unwrap(), b"readable");
            assert!(std::fs::write(readonly.join("value"), b"changed").is_err());
            std::fs::write(allowed.join("created"), b"ok").unwrap();
            assert!(std::fs::set_permissions(&allowed, std::fs::Permissions::from_mode(0o777)).is_err());
            assert!(std::fs::read(denied.join("secret")).is_err());
            assert!(UnixStream::connect(control).is_err());
            // Seatbelt denies access to ambient endpoints. A private
            // AF_UNIX socketpair carries no external authority and remains
            // available for worker-internal communication.
            let _private_pair = UnixStream::pair().unwrap();
            assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err());
            assert!(Command::new("/usr/bin/true").status().is_err());
            let parent = unsafe { libc::getppid() };
            assert_eq!(unsafe { libc::kill(parent, 0) }, -1, "worker signalled its parent");
        })
        .await
        .unwrap();
        tokio::task::spawn_blocking(|| 7usize).await.unwrap()
    });

    let mut request = [0; 4];
    inherited.read_exact(&mut request).unwrap();
    assert_eq!(&request, b"ping");
    inherited.write_all(b"pong").unwrap();
    std::process::exit(0);
}
