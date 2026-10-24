//! Exercise the real startup profile and attestation in an owned subprocess.

use super::*;
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};

// Match the gateway's single-threaded entry before libtest owns descriptors.
#[used]
#[unsafe(link_section = "__DATA,__mod_init_func")]
static GATEWAY_PROBE_ENTRY: extern "C" fn() = run_gateway_probe;

extern "C" fn run_gateway_probe() {
    if std::env::var_os("GATEWAY_EXEC_ERRNO_DIAGNOSTIC").is_some() {
        // SAFETY: same single-threaded startup and standard-stream boundary.
        unsafe { capsem_foundation::unix::fd::close_inherited_descriptors().unwrap() };
        actual_gateway_exec_errno_diagnostic();
        std::process::exit(0);
    }
    if std::env::var("GATEWAY_ATTESTATION_VARIANT").as_deref() == Ok("entry") {
        // SAFETY: this initializer runs before the test runner or runtime owns
        // descriptors; only the intentional standard-stream grant survives.
        unsafe { capsem_foundation::unix::fd::close_inherited_descriptors().unwrap() };
        actual_gateway_attestation_child();
        std::process::exit(0);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn actual_gateway_profile_attests_before_publishing_markers() {
    for variant in ["runtime", "cloexec_stdin", "unlinked_cwd", "entry", "logs"] {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let (mut parent, child) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let status = Command::new(sandbox_test_executable(&root))
            .args([
                "--exact",
                "tests::attestation::actual_gateway_attestation_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("GATEWAY_ATTESTATION_ROOT", &root)
            .env("GATEWAY_ATTESTATION_VARIANT", variant)
            .env("LLVM_PROFILE_FILE", root.join("profile"))
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(child)))
            .stdout(Stdio::from(std::fs::File::create(root.join("stdout")).unwrap()))
            .stderr(Stdio::from(std::fs::File::create(root.join("stderr")).unwrap()))
            .status()
            .unwrap();
        assert!(
            status.success(),
            "gateway startup attestation child {variant} failed: {status}\n{}\n{}",
            std::fs::read_to_string(root.join("stdout")).unwrap(),
            std::fs::read_to_string(root.join("stderr")).unwrap(),
        );
        let mut receipt = [0; 4];
        parent
            .read_exact(&mut receipt)
            .expect("exec success cannot impersonate an attestation receipt");
        assert_eq!(&receipt, b"safe");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn actual_gateway_attestation_child() {
    let Some(root) = std::env::var_os("GATEWAY_ATTESTATION_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    assert!(Command::new(EXECUTION_PROBE).status().unwrap().success());
    let auth = AuthState::prepare(&root).unwrap();
    let log = root.join("profile");
    std::fs::File::create(&log).unwrap();
    let uds = root.join("service.sock");
    let _uds = UnixListener::bind(&uds).unwrap();
    let tcp = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let socket = std::io::stdin();
    let mut receipt = UnixStream::from(capsem_foundation::unix::fd::duplicate(socket.as_fd()).unwrap());
    match std::env::var("GATEWAY_ATTESTATION_VARIANT").unwrap().as_str() {
        "cloexec_stdin" => {
            capsem_foundation::unix::fd::set_close_on_exec(socket.as_fd()).unwrap();
        }
        "unlinked_cwd" => {
            let cwd = root.join("cwd");
            std::fs::create_dir(&cwd).unwrap();
            std::env::set_current_dir(&cwd).unwrap();
            std::fs::remove_dir(&cwd).unwrap();
        }
        "runtime" | "entry" | "logs" => {}
        _ => panic!("unknown controlled variant"),
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        confine_gateway(&auth, &uds, tcp.local_addr().unwrap(), &log).unwrap();
    });
    receipt.write_all(b"safe").unwrap();
}

#[test]
fn nonexistent_executable_is_never_permission_denial() {
    let error = require_denied::<()>(
        Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
        "execute processes",
    )
    .unwrap_err();
    assert!(error.to_string().contains("did not prove"));
}

/// Run only in an explicitly isolated subprocess: exec success replaces it.
#[test]
fn actual_gateway_exec_errno_diagnostic() {
    use std::os::unix::process::CommandExt as _;
    if std::env::var_os("GATEWAY_EXEC_ERRNO_DIAGNOSTIC").is_none() {
        return;
    }
    assert!(Command::new(EXECUTION_PROBE).status().unwrap().success());
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let auth = AuthState::prepare(&root).unwrap();
    let log = root.join("gateway.log");
    std::fs::File::create(&log).unwrap();
    let uds = root.join("service.sock");
    let _uds = UnixListener::bind(&uds).unwrap();
    let tcp = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let gateway = runtime.block_on(async { confine_gateway(&auth, &uds, tcp.local_addr().unwrap(), &log) });
    let spawn = Command::new(EXECUTION_PROBE).status();
    let exec = Command::new(EXECUTION_PROBE).exec();
    println!(
        "GATEWAY_EXEC_DIAGNOSTIC gateway_ok={} spawn_errno={:?} exec_errno={:?}",
        gateway.is_ok(),
        spawn.as_ref().err().and_then(std::io::Error::raw_os_error),
        exec.raw_os_error(),
    );
    assert_eq!(exec.kind(), std::io::ErrorKind::PermissionDenied);
    gateway.unwrap();
}
