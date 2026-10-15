#![cfg(target_os = "linux")]
use super::*;
use capsem_foundation::{
    proctable::Process,
    unix::{
        fd,
        process::{ProcessHandle, ProcessId, Signal},
    },
};
use std::{
    io::{BufRead, BufReader},
    os::fd::AsFd,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

struct Orphan {
    handle: ProcessHandle,
    pid: u32,
    _input: std::process::ChildStdin,
}

impl Drop for Orphan {
    fn drop(&mut self) {
        let _ = self.handle.signal(Signal::Kill);
        let _ = self.handle.wait_until_exit(Instant::now() + Duration::from_secs(2));
    }
}

fn orphan(root: &Path, ignores_term: bool) -> Orphan {
    orphan_role(
        "capsem-process",
        "--session-dir",
        &root.join("sessions/original"),
        ignores_term,
    )
}

fn orphan_role(binary: &str, scope: &str, directory: &Path, ignores_term: bool) -> Orphan {
    let inner = if ignores_term {
        "trap '' TERM; printf '%s\\n' \"$$\"; read -r"
    } else {
        "printf '%s\\n' \"$$\"; read -r"
    };
    let mut parent = Command::new("bash")
        .args([
            "-c",
            "bash -c 'exec -a \"$3\" bash -c \"$1\" -- \"$4\" \"$2\"' -- \"$1\" \"$2\" \"$3\" \"$4\" <&0 &",
            "--",
            inner,
        ])
        .arg(directory)
        .arg(binary)
        .arg(scope)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = parent.stdin.take().unwrap();
    let output = parent.stdout.take().unwrap();
    assert!(fd::wait_readable(output.as_fd(), Duration::from_secs(2)).unwrap());
    let mut line = String::new();
    BufReader::new(output).read_line(&mut line).unwrap();
    let pid = line.trim().parse().unwrap();
    assert!(parent.wait().unwrap().success());
    let handle = ProcessHandle::open(ProcessId::try_from(pid).unwrap()).unwrap();
    Orphan {
        handle,
        pid,
        _input: input,
    }
}

#[test]
fn startup_retires_the_previous_gateway_before_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let gateway = orphan_role("capsem-gateway", "--run-dir", directory.path(), false);
    retire(directory.path()).unwrap();
    assert!(
        gateway.handle.has_exited().unwrap(),
        "old gateway must release its singleton before replacement"
    );
}

#[test]
fn startup_confirms_pinned_exit_when_fresh_argv_disappears() {
    let directory = tempfile::tempdir().unwrap();
    let child = orphan(directory.path(), false);
    let observed = proctable::processes()
        .unwrap()
        .into_iter()
        .find(|process| process.pid == child.pid)
        .unwrap();
    let target = Target {
        handle: ProcessHandle::open(ProcessId::try_from(child.pid).unwrap()).unwrap(),
        observed,
    };
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(20));
            child.handle.signal(Signal::Terminate).unwrap();
        });
        verify_candidates(vec![target], &[])
    });
    assert!(result.unwrap().is_empty());
    assert!(child
        .handle
        .wait_until_exit(Instant::now() + Duration::from_secs(2))
        .unwrap());
}

#[test]
fn startup_missing_argv_cannot_authorize_or_discard_a_live_process() {
    let directory = tempfile::tempdir().unwrap();
    let child = orphan(directory.path(), false);
    let observed = proctable::processes()
        .unwrap()
        .into_iter()
        .find(|process| process.pid == child.pid)
        .unwrap();
    let target = Target {
        handle: ProcessHandle::open(ProcessId::try_from(child.pid).unwrap()).unwrap(),
        observed,
    };
    assert!(verify_candidates(vec![target], &[]).is_err());
    assert!(!child.handle.has_exited().unwrap());
}

#[test]
fn startup_gateway_scope_is_exact_and_unique() {
    let root = Path::new("/tmp/startup-owned-root");
    for arguments in [
        vec!["--run-dir", "/tmp/startup-owned-root-other"],
        vec!["--run-dir", "/tmp/startup-owned-root/sessions/a"],
        vec![
            "--run-dir",
            "/tmp/startup-owned-root",
            "--run-dir",
            "/tmp/startup-owned-root",
        ],
        vec!["--session-dir", "/tmp/startup-owned-root"],
        vec!["--run-dir"],
    ] {
        let argv = std::iter::once("capsem-gateway")
            .chain(arguments)
            .map(std::ffi::OsString::from)
            .collect();
        let process = Process::from_argv(9001, 1, argv);
        assert!(super::super::find_orphan_capsem_pids(&[process], root).is_empty());
    }
}

#[test]
fn startup_discards_only_kernel_confirmed_absence_before_pinning() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    let observed = Process::from_argv(
        pid,
        1,
        vec![
            "capsem-process".into(),
            "--session-dir".into(),
            directory.path().join("sessions/original").into_os_string(),
        ],
    );
    assert!(pin_candidates(&[observed], directory.path()).unwrap().is_empty());
}

#[test]
fn startup_orphan_retirement_waits_for_original_native_exit() {
    let directory = tempfile::tempdir().unwrap();
    let child = orphan(directory.path(), false);
    retire(directory.path()).unwrap();
    assert!(child.handle.has_exited().unwrap());
}

#[test]
fn startup_orphan_retirement_forces_only_term_resistant_original_child() {
    let directory = tempfile::tempdir().unwrap();
    let child = orphan(directory.path(), true);
    let mut unrelated = Command::new("sleep").arg("30").spawn().unwrap();
    let result = retire(directory.path());
    let survived = unrelated.try_wait().unwrap().is_none();
    let _ = unrelated.kill();
    let _ = unrelated.wait();
    result.unwrap();
    assert!(child.handle.has_exited().unwrap());
    assert!(survived);
}

#[test]
fn startup_orphan_pin_revalidates_native_argv_and_rejects_stale_pid_observation() {
    let directory = tempfile::tempdir().unwrap();
    let original = orphan(directory.path(), false);
    let observed = capsem_foundation::proctable::processes()
        .unwrap()
        .into_iter()
        .find(|process| process.pid == original.pid)
        .unwrap();
    let other_root = tempfile::tempdir().unwrap();
    let unrelated = orphan(other_root.path(), false);
    // A stale table entry naming a PID now occupied by an unrelated native
    // process must not authorize that process; fresh kernel argv is real.
    let stale = Process::from_argv(unrelated.pid, observed.parent_pid, observed.argv);
    assert!(pin_candidates(&[stale], directory.path()).is_err());
    assert!(!unrelated.handle.has_exited().unwrap());
    assert!(!original.handle.has_exited().unwrap());
}

#[test]
fn startup_orphan_selection_does_not_adopt_current_service_children() {
    use std::os::unix::process::CommandExt;
    let directory = tempfile::tempdir().unwrap();
    let mut child = Command::new("bash")
        .arg0("capsem-process")
        .args(["-c", "printf '%s\\n' \"$$\"; read -r", "--", "--session-dir"])
        .arg(directory.path().join("sessions/current"))
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    assert!(fd::wait_readable(output.as_fd(), Duration::from_secs(2)).unwrap());
    let mut ready = String::new();
    BufReader::new(output).read_line(&mut ready).unwrap();
    let observed = capsem_foundation::proctable::processes()
        .unwrap()
        .into_iter()
        .find(|process| process.pid == child.id())
        .unwrap();
    assert_eq!(
        super::super::find_orphan_capsem_pids(std::slice::from_ref(&observed), directory.path()),
        vec![i32::try_from(child.id()).unwrap()]
    );
    let pinned = pin_candidates(&[observed], directory.path());
    let survived = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    assert!(pinned.unwrap().is_empty());
    assert!(survived);
}

#[test]
fn startup_orphan_selection_refuses_owner_still_parented_to_original_service() {
    let directory = tempfile::tempdir().unwrap();
    let mut parent = Command::new("bash")
        .args(["-c", "bash -c 'exec -a capsem-process bash -c \"$1\" -- --session-dir \"$2\" --parent-pid \"$PPID\"' -- 'printf \"%s\\n\" \"$$\"; read -r' \"$1\" <&0 & read -r", "--"])
        .arg(directory.path().join("sessions/original"))
        .env_clear().env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    let input = parent.stdin.take().unwrap();
    let output = parent.stdout.take().unwrap();
    assert!(fd::wait_readable(output.as_fd(), Duration::from_secs(2)).unwrap());
    let mut ready = String::new();
    BufReader::new(output).read_line(&mut ready).unwrap();
    let pid = ready.trim().parse().unwrap();
    let child = Orphan {
        handle: ProcessHandle::open(ProcessId::try_from(pid).unwrap()).unwrap(),
        pid,
        _input: input,
    };
    let observed = capsem_foundation::proctable::processes()
        .unwrap()
        .into_iter()
        .find(|process| process.pid == pid)
        .unwrap();
    assert_eq!(observed.parent_pid, parent.id());
    let pinned = pin_candidates(&[observed], directory.path());
    let survived = !child.handle.has_exited().unwrap();
    parent.kill().unwrap();
    parent.wait().unwrap();
    assert!(
        pinned.is_err(),
        "startup adopted an owner whose original service is alive"
    );
    assert!(survived);
}
