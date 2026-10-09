use super::*;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::fd::AsFd;
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
    assert_eq!(&reply, b"pong");
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
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    confine(
        &Policy::new(Role::Ledger)
            .allow(&allowed, Access::ReadWrite)
            .allow(&readonly, Access::ReadOnly)
            .allow(&exact, Access::ReadOnly),
    )
    .unwrap();

    runtime.block_on(async {
        tokio::task::spawn_blocking(move || {
            assert_eq!(std::fs::read(readonly.join("value")).unwrap(), b"readable");
            assert_eq!(std::fs::read(exact).unwrap(), b"one file");
            assert!(std::fs::write(readonly.join("value"), b"changed").is_err());
            std::fs::write(allowed.join("created"), b"ok").unwrap();
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
