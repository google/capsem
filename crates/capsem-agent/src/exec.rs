//! Guest exec lifecycle and typed bidirectional stream ownership.

use crate::control_writer::CtrlSender;
use crate::send_guest_msg;
use crate::vsock_io::{vsock_connect, VSOCK_HOST_CID};
use capsem_foundation::unix::{
    fd::{shutdown, SocketShutdown},
    process::{send_process_group_signal, ProcessId, Signal},
};
use capsem_proto::{GuestToHost, VSOCK_PORT_EXEC};
use std::io::{self, Read as _, Write as _};
use std::os::fd::{AsFd, FromRawFd, RawFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::thread;

/// Maximum vsock_connect attempts when the host returns ECONNRESET, e.g.
/// briefly after `restoreMachineStateFromURL` while the kernel-side
/// accept queue is still settling. 5 attempts × ECONNRESET_BACKOFF_MS
/// keeps the transient retry short without hiding real connect failures.
const ECONNRESET_MAX_ATTEMPTS: usize = 5;
const ECONNRESET_BACKOFF_MS: u64 = 20;

/// Connect via the supplied closure, retrying on ECONNRESET only.
/// All other error kinds bail immediately so we don't paper over real
/// misconfiguration (refused, address-family-unsupported, etc.).
///
/// Bug C: post-`restoreState` the agent's `vsock_connect` to host port
/// 5005 (EXEC) can transiently see ECONNRESET while the kernel-side
/// accept queue is still attaching to the freshly-registered VZ
/// listener. A single-shot connect failed -> run_exec returned 126 ->
/// `exec_done` cached the bad code -> every host retry/replay was
/// poisoned. The retry isolates this transient transport state.
fn vsock_connect_with_econnreset_retry<F>(mut connect_fn: F) -> io::Result<RawFd>
where
    F: FnMut() -> io::Result<RawFd>,
{
    let mut last_err = None;
    for attempt in 1..=ECONNRESET_MAX_ATTEMPTS {
        match connect_fn() {
            Ok(fd) => return Ok(fd),
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {
                last_err = Some(e);
                if attempt < ECONNRESET_MAX_ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_millis(ECONNRESET_BACKOFF_MS));
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::from(io::ErrorKind::ConnectionReset)))
}

#[derive(Debug, Default)]
pub(super) struct ExecCancellation {
    state: std::sync::Mutex<ExecCancellationState>,
}

#[derive(Debug, Default)]
struct ExecCancellationState {
    pid: Option<ProcessId>,
    cancelled: bool,
    completed: bool,
}

impl ExecCancellation {
    fn attach(self: &std::sync::Arc<Self>, pid: ProcessId) {
        let cancelled = {
            let mut state = self.state.lock().unwrap();
            state.pid = Some(pid);
            state.cancelled
        };
        if cancelled {
            self.terminate(pid);
        }
    }

    pub(super) fn cancel(self: &std::sync::Arc<Self>) {
        let pid = {
            let mut state = self.state.lock().unwrap();
            state.cancelled = true;
            (!state.completed).then_some(state.pid).flatten()
        };
        if let Some(pid) = pid {
            self.terminate(pid);
        }
    }

    fn terminate(self: &std::sync::Arc<Self>, pid: ProcessId) {
        let _ = send_process_group_signal(pid, Signal::Terminate);
        let cancellation = std::sync::Arc::clone(self);
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_secs(1));
            let should_kill = {
                let state = cancellation.state.lock().unwrap();
                !state.completed && state.pid == Some(pid)
            };
            if should_kill {
                let _ = send_process_group_signal(pid, Signal::Kill);
            }
        });
    }

    fn complete(&self) {
        let mut state = self.state.lock().unwrap();
        state.completed = true;
        state.pid = None;
    }
}

/// Outcome of a `run_exec` call. Distinguishes a real child exit
/// (cache it for dedup-replay on host duplicate Exec delivery) from a
/// transport failure that never reached the child (do NOT cache --
/// the next host replay deserves a fresh attempt against a possibly
/// recovered transport).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExecOutcome {
    /// Child process ran to completion with `i32` exit code.
    Done(i32),
    /// vsock_connect to the host EXEC port exhausted retries; ExecStarted
    /// or any subsequent step never landed. The host still gets an
    /// ExecDone {exit_code: 126} (so the caller sees a result), but the
    /// agent does not poison `exec_done` with this transient.
    TransportFailed,
}

impl ExecOutcome {
    pub(super) fn should_cache(&self) -> bool {
        matches!(self, ExecOutcome::Done(_))
    }

    pub(super) fn exit_code(&self) -> i32 {
        match self {
            ExecOutcome::Done(code) => *code,
            ExecOutcome::TransportFailed => 126,
        }
    }
}

/// Execute a command as a direct child process, streaming output over vsock:5005.
///
/// Runs in a background thread so control_loop remains responsive to heartbeats.
/// Typed output frames flow on a dedicated exec vsock connection. The exit
/// code is sent as ExecDone via the serialized control write channel.
pub(super) fn run_exec(
    ctrl_tx: &CtrlSender,
    id: u64,
    command: &str,
    boot_env: &[(String, String)],
    cancellation: &std::sync::Arc<ExecCancellation>,
) -> ExecOutcome {
    // Connect to host exec port. Retry on ECONNRESET only -- post-restore
    // VZ transient (Bug C). Other errors bail immediately.
    let exec_fd = match vsock_connect_with_econnreset_retry(|| vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_EXEC)) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("[capsem-agent] exec[{id}] vsock connect failed: {e}");
            let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code: 126 });
            return ExecOutcome::TransportFailed;
        }
    };

    ExecOutcome::Done(run_exec_on_fds_with_cancel(
        exec_fd,
        ctrl_tx,
        id,
        command,
        boot_env,
        cancellation,
    ))
}

/// Inner exec implementation that takes pre-connected fds (testable without vsock).
/// `ctrl_tx` serializes writes to the control channel (prevents frame corruption
/// from concurrent writers). `exec_fd` is consumed: closed on all exit paths.
#[cfg(test)]
pub(super) fn run_exec_on_fds(
    exec_fd: RawFd,
    ctrl_tx: &CtrlSender,
    id: u64,
    command: &str,
    boot_env: &[(String, String)],
) -> i32 {
    run_exec_on_fds_with_cancel(
        exec_fd,
        ctrl_tx,
        id,
        command,
        boot_env,
        &std::sync::Arc::new(ExecCancellation::default()),
    )
}

pub(super) fn run_exec_on_fds_with_cancel(
    exec_fd: RawFd,
    ctrl_tx: &CtrlSender,
    id: u64,
    command: &str,
    boot_env: &[(String, String)],
    cancellation: &std::sync::Arc<ExecCancellation>,
) -> i32 {
    // Send ExecStarted handshake so host knows which exec ID this connection belongs to.
    if let Err(e) = send_guest_msg(
        exec_fd,
        &GuestToHost::ExecStarted {
            id,
            output_protocol: capsem_proto::ExecOutputProtocol::FramedLanes,
        },
    ) {
        eprintln!("[capsem-agent] exec[{id}] handshake failed: {e}");
        let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code: 126 });
        return 126;
    }

    // Own the exec socket after the control handshake. Clones share one
    // bidirectional connection while keeping input and output independently
    // interruptible.
    let exec = unsafe { std::fs::File::from_raw_fd(exec_fd) };
    let input = match exec.try_clone() {
        Ok(input) => input,
        Err(error) => {
            eprintln!("[capsem-agent] exec[{id}] input clone failed: {error}");
            let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code: 126 });
            return 126;
        }
    };
    let output = std::sync::Arc::new(std::sync::Mutex::new(exec));

    // Spawn child process with piped stdout and stderr.
    let cwd = default_exec_cwd();
    let mut child = match std::process::Command::new("bash")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(cwd)
        .envs(boot_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .process_group(0)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[capsem-agent] exec[{id}] spawn failed: {e}");
            let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code: 126 });
            return 126;
        }
    };
    let child_pid = match ProcessId::try_from(child.id()) {
        Ok(pid) => pid,
        Err(error) => {
            eprintln!("[capsem-agent] exec[{id}] invalid child pid: {error}");
            let _ = child.kill();
            let _ = child.wait();
            let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code: 126 });
            return 126;
        }
    };
    cancellation.attach(child_pid);

    let stdin_thread = child.stdin.take().map(|mut stdin| {
        let mut input = input;
        thread::spawn(move || {
            while let Ok(capsem_proto::ExecInputFrame::Data(data)) = capsem_proto::read_exec_input(&mut input) {
                if stdin.write_all(&data).is_err() || stdin.flush().is_err() {
                    break;
                }
            }
        })
    });

    // Serialize complete typed frames so stdout and stderr retain their lanes
    // without interleaving frame bytes from the two reader threads.
    let stderr_thread = child.stderr.take().map(|mut stderr| {
        let output = std::sync::Arc::clone(&output);
        thread::spawn(move || {
            let mut buf = vec![0u8; EXEC_OUTPUT_READ_BYTES];
            loop {
                match stderr.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let channel = capsem_proto::ExecOutputChannel::Stderr;
                        if capsem_proto::write_exec_output_data(&mut *output.lock().unwrap(), channel, &buf[..n])
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
    });

    let stdout_thread = child.stdout.take().map(|mut stdout| {
        let output = std::sync::Arc::clone(&output);
        thread::spawn(move || {
            let mut buf = vec![0u8; EXEC_OUTPUT_READ_BYTES];
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let channel = capsem_proto::ExecOutputChannel::Stdout;
                        if capsem_proto::write_exec_output_data(&mut *output.lock().unwrap(), channel, &buf[..n])
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        })
    });

    // Drain before reaping. A grandchild holding the stdout pipe keeps these
    // joins blocked, and while the child is unreaped its pid is still its
    // process group's, so a cancellation arriving during the drain can still
    // signal the group. Completing first made cancel() a no-op and ExecDone
    // waited for the grandchild.
    finish_exec_io(&output, stderr_thread, stdout_thread, stdin_thread);

    let status = child.wait();
    cancellation.complete();

    let exit_code = match status {
        Ok(status) => status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(1)),
        Err(_) => 126,
    };

    drop(output);

    // Send ExecDone via serialized control write channel.
    eprintln!("[capsem-agent] exec[{id}] done: exit_code={exit_code}");
    let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code });
    exit_code
}

/// One read of a child's stdout or stderr, sent as one frame. An 8 KiB buffer
/// turned 256 KiB of output into 32 frames, each copied and written apart.
const EXEC_OUTPUT_READ_BYTES: usize = 64 * 1024;
const _: () = assert!(EXEC_OUTPUT_READ_BYTES <= capsem_proto::MAX_EXEC_DATA_BYTES);

fn finish_exec_io(
    output: &std::sync::Arc<std::sync::Mutex<std::fs::File>>,
    stderr_thread: Option<std::thread::JoinHandle<()>>,
    stdout_thread: Option<std::thread::JoinHandle<()>>,
    stdin_thread: Option<std::thread::JoinHandle<()>>,
) {
    // AF_VSOCK does not provide a portable half-close. Shutting down reads
    // while an output worker is still writing can make the host observe EOF
    // before the final frames. Drain both output pipes first, then wake a
    // blocked input reader after no more guest output remains to publish.
    if let Some(thread) = stderr_thread {
        let _ = thread.join();
    }
    if let Some(thread) = stdout_thread {
        let _ = thread.join();
    }
    let _ = shutdown(output.lock().unwrap().as_fd(), SocketShutdown::Read);
    if let Some(thread) = stdin_thread {
        let _ = thread.join();
    }
}

pub(super) fn default_exec_cwd() -> &'static str {
    if capsem_foundation::unix::process::current_uid() == 0 && std::path::Path::new("/root").is_dir() {
        "/root"
    } else {
        "/"
    }
}

#[cfg(test)]
mod tests;
