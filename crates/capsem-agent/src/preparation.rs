//! Startup-only mode of the existing native guest helper. No host transport.

use capsem_proto::preparation::{self, PreparationFailure as Failure, PreparationStatus as Status};
use nix::libc;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "preparation/privacy.rs"]
mod privacy;
#[path = "preparation/signals.rs"]
mod signals;

struct Input<'a> {
    timeout: Duration,
    script: &'a str,
    argv: &'a [String],
}
impl<'a> Input<'a> {
    fn parse(args: &'a [String]) -> Result<Self, Failure> {
        if args.len() < 4
            || args[2] != "--"
            || args[3].is_empty()
            || args[1].len() > capsem_proto::ENVIRONMENT_MAX_STARTUP_SCRIPT_BYTES
            || args.iter().any(|value| value.contains('\0'))
        {
            return Err(Failure::InvalidInput);
        }
        let seconds: u32 = args[0].parse().map_err(|_| Failure::InvalidInput)?;
        if seconds == 0 {
            return Err(Failure::InvalidInput);
        }
        Ok(Self {
            timeout: Duration::from_secs(u64::from(seconds)),
            script: &args[1],
            argv: &args[3..],
        })
    }
}

/// Own a valid private descriptor and close it on both child and entrypoint
/// exec. Script stdio receives only duplicates of the output descriptor.
fn private_fd(fd: RawFd) -> io::Result<File> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // F_GETFD established that this descriptor is open. This startup-only
    // invocation takes ownership once, before spawning any child.
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn report(status: &mut File, event: Status) -> io::Result<()> {
    status.write_all(&preparation::encode(&event)?)
}
fn exit_code(failure: Failure) -> i32 {
    match failure {
        Failure::Exit { code } => code,
        Failure::Signal { signal } => 128 + signal,
        Failure::TimedOut => 124,
        _ => 126,
    }
}
fn failed(status: &mut File, failure: Failure) -> i32 {
    let _ = report(status, Status::Failed { failure });
    exit_code(failure)
}

/// This helper is OCI PID1, under the original process user/cwd/env/filter.
/// No script, output or OS error details enter diagnostics or status records.
pub(crate) fn run(args: &[String]) -> i32 {
    let Ok(mut status) = private_fd(preparation::STATUS_FD) else {
        return 126;
    };
    let input = match Input::parse(args) {
        Ok(input) => input,
        Err(failure) => return failed(&mut status, failure),
    };
    let output = match private_fd(preparation::OUTPUT_FD) {
        Ok(output) => output,
        Err(_) => return failed(&mut status, Failure::InvalidInput),
    };
    let _privacy = match privacy::Guard::install() {
        Ok(privacy) => privacy,
        Err(_) => return failed(&mut status, Failure::Spawn),
    };
    let signals = match signals::Guard::install() {
        Ok(signals) => signals,
        Err(_) => return failed(&mut status, Failure::Spawn),
    };
    if report(&mut status, Status::Preparing).is_err() {
        return 126;
    }
    if let Err(failure) = supervise(&input, &output) {
        return failed(&mut status, failure);
    }
    if report(&mut status, Status::Entrypoint).is_err() {
        return 126;
    }
    // Restore inherited signal dispositions and close the separate output
    // rail. The original OCI stdin/stdout/stderr remain unchanged for exec.
    drop(signals);
    drop(output);
    let _error = Command::new(&input.argv[0]).args(&input.argv[1..]).exec();
    failed(&mut status, Failure::EntrypointExec)
}

fn supervise(input: &Input<'_>, output: &File) -> Result<(), Failure> {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(input.script)
        .arg("capsem-startup")
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(|_| Failure::Spawn)?)
        .stderr(output.try_clone().map_err(|_| Failure::Spawn)?)
        .process_group(0);
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Failure::ShellUnavailable
        } else {
            Failure::Spawn
        }
    })?;
    let deadline = Instant::now() + input.timeout;
    loop {
        if let Some(signal) = signals::received() {
            terminate(&mut child, signal);
            return Err(Failure::Signal { signal });
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                terminate(&mut child, libc::SIGKILL);
                return Err(match status.code() {
                    Some(code) => Failure::Exit { code },
                    None => Failure::Signal {
                        signal: status.signal().unwrap_or(libc::SIGKILL),
                    },
                });
            }
            Ok(None) => {}
            Err(_) => {
                terminate(&mut child, libc::SIGKILL);
                return Err(Failure::Wait);
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            terminate(&mut child, libc::SIGKILL);
            return Err(Failure::TimedOut);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

fn terminate(child: &mut Child, signal: i32) {
    // The shell has its own process group. The wrapper is PID1 in the OCI
    // namespace; exiting after failure also retires descendants that changed
    // groups. Never signal the wrapper's inherited group.
    let group = -(child.id() as i32);
    unsafe {
        libc::kill(group, signal);
    }
    if signal != libc::SIGKILL {
        std::thread::sleep(Duration::from_millis(100));
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
    }
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_arguments_require_a_deadline_script_and_literal_entrypoint() {
        let args = ["1", "printf 'approved script'", "--", "/entrypoint", "a b", "--help"].map(String::from);
        let parsed = Input::parse(&args).unwrap();
        assert_eq!(parsed.timeout.as_secs(), 1);
        assert_eq!(parsed.script, args[1]);
        assert_eq!(parsed.argv, &args[3..]);
        for invalid in [
            vec![],
            vec!["0", "x", "--", "/bin/true"],
            vec!["1", "x", "--"],
            vec!["1", "x", "other", "/bin/true"],
        ] {
            let invalid: Vec<String> = invalid.into_iter().map(String::from).collect();
            assert!(Input::parse(&invalid).is_err());
        }
    }
}
