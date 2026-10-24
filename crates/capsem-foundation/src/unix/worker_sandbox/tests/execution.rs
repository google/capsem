use super::*;
use nix::sys::signal::{signal, SigHandler, Signal};

#[used]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
static EXECUTION_PROBE_ENTRY: extern "C" fn() = run_execution_probe;

extern "C" fn run_execution_probe() {
    if std::env::var_os("EXECUTION_ATTESTATION_CHILD").is_some() {
        execution_attestation_child();
        std::process::exit(0);
    }
}

#[test]
fn denied_execution_preserves_cwd_signal_disposition_and_granted_io() {
    let root = tempfile::tempdir().unwrap();
    let (mut parent, child) = UnixStream::pair().unwrap();
    parent.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let executable = crate::sandbox_test_executable(root.path(), "capsem-worker-confinement-test");
    let mut child = crate::isolated_test_command(executable)
        .env("EXECUTION_ATTESTATION_CHILD", root.path())
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(child)))
        .spawn()
        .unwrap();
    parent.write_all(b"ping").unwrap();
    let mut reply = [0; 4];
    let received = parent.read_exact(&mut reply);
    assert!(child.wait().unwrap().success());
    received.unwrap();
    assert_eq!(&reply, b"pong");
}

#[test]
fn successful_execution_cannot_return_a_false_denial_receipt() {
    let root = tempfile::tempdir().unwrap();
    let (mut parent, child) = UnixStream::pair().unwrap();
    parent.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut child = crate::isolated_test_command(std::env::current_exe().unwrap())
        .env("EXECUTION_ATTESTATION_CHILD", root.path())
        .env("EXECUTION_ATTESTATION_UNCONFINED", "1")
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(child)))
        .spawn()
        .unwrap();
    assert!(child.wait().unwrap().success());
    let mut reply = Vec::new();
    parent.read_to_end(&mut reply).unwrap();
    assert!(
        reply.is_empty(),
        "execution replaced the child before any denial receipt"
    );
}

#[test]
fn only_actual_permission_denial_proves_execution_confinement() {
    for kind in [
        std::io::ErrorKind::NotFound,
        std::io::ErrorKind::InvalidInput,
        std::io::ErrorKind::Other,
    ] {
        assert!(require_execution_denial(std::io::Error::from(kind)).is_err());
    }
    require_execution_denial(std::io::Error::from(std::io::ErrorKind::PermissionDenied)).unwrap();
}

#[test]
fn execution_attestation_child() {
    let Some(root) = std::env::var_os("EXECUTION_ATTESTATION_CHILD") else {
        return;
    };
    let root = std::fs::canonicalize(root).unwrap();
    std::env::set_current_dir(&root).unwrap();
    let mut report = UnixStream::from(super::super::super::fd::duplicate(std::io::stdin().as_fd()).unwrap());
    if std::env::var_os("EXECUTION_ATTESTATION_UNCONFINED").is_some() {
        attest_execution_denied().unwrap();
        report.write_all(b"false denial receipt").unwrap();
        return;
    }
    // SAFETY: this explicit isolated child is single-threaded at process entry;
    // SIG_IGN is a valid disposition, not a Rust callback.
    unsafe { signal(Signal::SIGPIPE, SigHandler::SigIgn).unwrap() };
    confine(&Policy::new(Role::Gateway).allow(&root, Access::ReadWrite)).unwrap();
    attest_execution_denied().unwrap();
    assert_eq!(std::env::current_dir().unwrap(), root);
    // SAFETY: same isolated process and valid disposition. The return value
    // proves the failed execution attempt did not reset the runtime's SIGPIPE.
    assert_eq!(
        unsafe { signal(Signal::SIGPIPE, SigHandler::SigIgn).unwrap() },
        SigHandler::SigIgn
    );
    let mut request = [0; 4];
    report.read_exact(&mut request).unwrap();
    assert_eq!(&request, b"ping");
    report.write_all(b"pong").unwrap();
}
