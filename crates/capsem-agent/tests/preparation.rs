//! Actual native helper, private pipes, real shell/exec and cancellation.
use capsem_proto::preparation::{decode, PreparationFailure as Failure, PreparationStatus as Status, MAX_STATUS_BYTES};
use nix::libc;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};

fn pipe() -> (File, File) {
    let mut descriptors = [0; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let read = unsafe { File::from_raw_fd(descriptors[0]) };
    let write = unsafe { File::from_raw_fd(descriptors[1]) };
    // Keep child dup2 targets disjoint from both source descriptors.
    let high = unsafe { libc::fcntl(write.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
    assert!(high >= 10);
    drop(write);
    (read, unsafe { File::from_raw_fd(high) })
}
fn spawn(script: &str, argv: &[&str], timeout: u32) -> (Child, File, File) {
    spawn_in(script, argv, timeout, std::path::Path::new("."))
}
fn spawn_in(script: &str, argv: &[&str], timeout: u32, cwd: &std::path::Path) -> (Child, File, File) {
    let timeout = timeout.to_string();
    let mut arguments = vec![timeout.as_str(), script, "--"];
    arguments.extend_from_slice(argv);
    spawn_arguments(&arguments, cwd)
}

fn spawn_arguments(arguments: &[&str], cwd: &std::path::Path) -> (Child, File, File) {
    let (output_read, output_write) = pipe();
    let (status_read, status_write) = pipe();
    let output = output_write.as_raw_fd();
    let status = status_write.as_raw_fd();
    let mut command = Command::new(env!("CARGO_BIN_EXE_capsem-sysutil"));
    command
        .current_dir(cwd)
        .arg0("capsem-prepare")
        .args(arguments)
        .env("PREPARATION_TEST_VARIABLE", "approved ordinary variable")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(output, capsem_proto::preparation::OUTPUT_FD) < 0
                || libc::dup2(status, capsem_proto::preparation::STATUS_FD) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().unwrap();
    drop(output_write);
    drop(status_write);
    (child, output_read, status_read)
}
fn event(input: &mut File) -> Status {
    let mut length = [0; 4];
    input.read_exact(&mut length).unwrap();
    let length = u32::from_be_bytes(length);
    assert!(length > 0 && length <= MAX_STATUS_BYTES);
    let mut payload = vec![0; length as usize];
    input.read_exact(&mut payload).unwrap();
    decode(&payload).unwrap()
}
fn eof(input: &mut File) {
    let mut byte = [0];
    assert_eq!(input.read(&mut byte).unwrap(), 0);
}
fn text(mut input: File) -> String {
    let mut bytes = String::new();
    input.read_to_string(&mut bytes).unwrap();
    bytes
}

#[cfg(target_os = "linux")]
#[test]
fn approved_repository_preparation_preserves_existing_workspace_contents() {
    let workspace = tempfile::tempdir().unwrap();
    let repo = workspace.path().join("nested/project");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let origin = "https://example.com/approved.git";
    assert!(Command::new("git")
        .args(["remote", "add", "origin", origin])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    std::fs::write(repo.join("AGENTS.md"), "keep existing instructions").unwrap();
    let mut arguments = [
        "--repositories",
        workspace.path().to_str().unwrap(),
        "1",
        origin,
        "0123456789012345678901234567890123456789",
        "nested/project",
        "0",
        "",
        "--",
        "/bin/true",
    ];
    let (child, output, mut status) = spawn_arguments(&arguments, workspace.path());
    assert_eq!(event(&mut status), Status::Preparing);
    assert_eq!(event(&mut status), Status::Entrypoint);
    eof(&mut status);
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(text(output), "");
    arguments[3] = "https://example.com/different.git";
    let (child, output, mut status) = spawn_arguments(&arguments, workspace.path());
    assert_eq!(event(&mut status), Status::Preparing);
    assert_eq!(
        event(&mut status),
        Status::Failed {
            failure: Failure::RepositoryDestination
        }
    );
    eof(&mut status);
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(126));
    assert_eq!(text(output), "");
    assert_eq!(
        std::fs::read_to_string(repo.join("AGENTS.md")).unwrap(),
        "keep existing instructions"
    );
}

#[test]
fn startup_exec_preserves_pid_arguments_environment_and_separate_output() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("prepared");
    let script = format!(
        "printf '%s' \"$PREPARATION_TEST_VARIABLE\"; printf ready > '{}'",
        marker.display()
    );
    let actual = "test \"$(cat \"$1\")\" = ready || exit 41; printf '%s:%s:%s:%s' \"$$\" \"$2\" \"$3\" \"$PWD\"";
    let literal = "a b;'$ --help";
    let (child, output, mut status) = spawn_in(
        &script,
        &[
            "/bin/sh",
            "-c",
            actual,
            "entrypoint",
            marker.to_str().unwrap(),
            literal,
            "--help",
        ],
        5,
        directory.path(),
    );
    let pid = child.id();
    assert_eq!(event(&mut status), Status::Preparing);
    assert_eq!(event(&mut status), Status::Entrypoint);
    eof(&mut status);
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert_eq!(
        result.stdout,
        format!("{pid}:{literal}:--help:{}", directory.path().display()).as_bytes()
    );
    assert!(result.stderr.is_empty());
    assert_eq!(text(output), "approved ordinary variable");
}

#[test]
fn startup_failure_and_timeout_block_entrypoint() {
    for (script, code, failure) in [
        ("printf startup-failure; exit 17", 17, Failure::Exit { code: 17 }),
        ("sleep 30 & wait", 124, Failure::TimedOut),
    ] {
        let started = std::time::Instant::now();
        let (child, output, mut status) = spawn(script, &["/bin/sh", "-c", "printf ENTRYPOINT_MUST_NOT_RUN"], 1);
        assert_eq!(event(&mut status), Status::Preparing);
        assert_eq!(event(&mut status), Status::Failed { failure });
        eof(&mut status);
        let result = child.wait_with_output().unwrap();
        assert_eq!(result.status.code(), Some(code));
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        assert!(!text(output).contains("ENTRYPOINT_MUST_NOT_RUN"));
    }
}

#[test]
fn startup_cancellation_and_entrypoint_signals_keep_the_original_process() {
    let (child, output, mut status) = spawn("exec sleep 30", &["/bin/sleep", "30"], 60);
    assert_eq!(event(&mut status), Status::Preparing);
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    assert_eq!(
        event(&mut status),
        Status::Failed {
            failure: Failure::Signal { signal: libc::SIGTERM }
        }
    );
    eof(&mut status);
    assert_eq!(
        child.wait_with_output().unwrap().status.code(),
        Some(128 + libc::SIGTERM)
    );
    assert!(text(output).is_empty());

    let (child, output, mut status) = spawn(":", &["/bin/sleep", "30"], 5);
    assert_eq!(event(&mut status), Status::Preparing);
    assert_eq!(event(&mut status), Status::Entrypoint);
    eof(&mut status);
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    assert_eq!(child.wait_with_output().unwrap().status.signal(), Some(libc::SIGTERM));
    assert!(text(output).is_empty());
}

#[test]
fn startup_cannot_inherit_or_reopen_the_private_status_descriptor() {
    let script = "if (printf forged >&4) 2>/dev/null; then exit 51; fi; if (printf forged > /proc/$PPID/fd/4) 2>/dev/null; then exit 52; fi; printf private-status-held";
    let (child, output, mut status) = spawn(script, &["/bin/true"], 5);
    assert_eq!(event(&mut status), Status::Preparing);
    assert_eq!(event(&mut status), Status::Entrypoint);
    eof(&mut status);
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(text(output), "private-status-held");
}

#[test]
fn startup_reports_exec_failure_without_sending_argv_or_script() {
    let (child, output, mut status) = spawn(":", &["/missing-entrypoint-sensitive-argument"], 5);
    assert_eq!(event(&mut status), Status::Preparing);
    assert_eq!(event(&mut status), Status::Entrypoint);
    assert_eq!(
        event(&mut status),
        Status::Failed {
            failure: Failure::EntrypointExec
        }
    );
    eof(&mut status);
    let result = child.wait_with_output().unwrap();
    assert_eq!(result.status.code(), Some(126));
    assert!(result.stdout.is_empty() && result.stderr.is_empty());
    assert!(text(output).is_empty());
}
