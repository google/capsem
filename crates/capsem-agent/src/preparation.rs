//! Startup-only mode of the existing native guest helper. No host transport.

use capsem_foundation::unix::{
    fd,
    process::{self, cancellation, Signal},
};
use capsem_proto::preparation::{self, PreparationFailure as Failure, PreparationStatus as Status};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::RawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
#[path = "preparation/repositories.rs"]
mod repositories;

struct Input<'a> {
    startup: Option<(Duration, &'a str)>,
    #[cfg(target_os = "linux")]
    repositories: Option<(&'a std::path::Path, Vec<repositories::Repository<'a>>)>,
    argv: &'a [String],
}
impl<'a> Input<'a> {
    fn parse(args: &'a [String]) -> Result<Self, Failure> {
        if args.iter().any(|value| value.contains('\0')) {
            return Err(Failure::InvalidInput);
        }
        #[cfg(target_os = "linux")]
        let (args, repositories) = if args.first().is_some_and(|arg| arg == "--repositories") {
            let workspace = std::path::Path::new(args.get(1).ok_or(Failure::InvalidInput)?);
            let count: usize = args
                .get(2)
                .and_then(|count| count.parse().ok())
                .ok_or(Failure::InvalidInput)?;
            if !workspace.is_absolute() || !(1..=capsem_proto::ENVIRONMENT_MAX_REPOSITORIES).contains(&count) {
                return Err(Failure::InvalidInput);
            }
            let end = 3 + count * 3;
            let repositories = args
                .get(3..end)
                .ok_or(Failure::InvalidInput)?
                .chunks_exact(3)
                .map(repositories::Repository::parse)
                .collect::<Result<Vec<_>, _>>()?;
            (&args[end..], Some((workspace, repositories)))
        } else {
            (args, None)
        };
        if args.len() < 4
            || args[2] != "--"
            || args[3].is_empty()
            || args[1].len() > capsem_proto::ENVIRONMENT_MAX_STARTUP_SCRIPT_BYTES
        {
            return Err(Failure::InvalidInput);
        }
        let seconds: u32 = args[0].parse().map_err(|_| Failure::InvalidInput)?;
        if seconds == 0 {
            #[cfg(target_os = "linux")]
            if repositories.is_some() && args[1].is_empty() {
                return Ok(Self {
                    startup: None,
                    repositories,
                    argv: &args[3..],
                });
            }
            return Err(Failure::InvalidInput);
        }
        Ok(Self {
            startup: Some((Duration::from_secs(u64::from(seconds)), &args[1])),
            #[cfg(target_os = "linux")]
            repositories,
            argv: &args[3..],
        })
    }
}

/// Own a valid private descriptor and close it on both child and entrypoint
/// exec. Script stdio receives only duplicates of the output descriptor.
fn private_fd(fd: RawFd) -> io::Result<File> {
    // This startup-only invocation owns the inherited grant once, before any
    // child or other thread can reuse it. Foundation validates it before taking
    // ownership and setting CLOEXEC.
    unsafe { fd::take_inherited_close_on_exec(fd) }.map(File::from)
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
    #[cfg(target_os = "linux")]
    if process::disable_dumpability().is_err() {
        return failed(&mut status, Failure::Spawn);
    }
    let signals = match cancellation::Guard::install() {
        Ok(signals) => signals,
        Err(_) => return failed(&mut status, Failure::Spawn),
    };
    if report(&mut status, Status::Preparing).is_err() {
        return 126;
    }
    #[cfg(target_os = "linux")]
    if let Some((workspace, repositories)) = &input.repositories {
        if let Err(failure) = repositories::prepare(workspace, repositories, &signals) {
            return failed(&mut status, failure);
        }
    }
    if let Some((timeout, script)) = input.startup {
        if let Err(failure) = run_startup(script, timeout, &output, &signals) {
            return failed(&mut status, failure);
        }
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

fn run_startup(script: &str, timeout: Duration, output: &File, signals: &cancellation::Guard) -> Result<(), Failure> {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(script)
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
    supervise_child(&mut child, Instant::now() + timeout, signals)
}

fn supervise_child(child: &mut Child, deadline: Instant, signals: &cancellation::Guard) -> Result<(), Failure> {
    loop {
        if let Some(signal) = signals.received() {
            terminate(child, signal);
            return Err(Failure::Signal {
                signal: signal.number(),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                terminate(child, Signal::Kill);
                return Err(match status.code() {
                    Some(code) => Failure::Exit { code },
                    None => Failure::Signal {
                        signal: status.signal().unwrap_or(Signal::Kill.number()),
                    },
                });
            }
            Ok(None) => {}
            Err(_) => {
                terminate(child, Signal::Kill);
                return Err(Failure::Wait);
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            terminate(child, Signal::Kill);
            return Err(Failure::TimedOut);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

fn terminate(child: &mut Child, signal: Signal) {
    // The shell has its own process group. The wrapper is PID1 in the OCI
    // namespace; exiting after failure also retires descendants that changed
    // groups. Never signal the wrapper's inherited group.
    if let Ok(group) = process::ProcessId::try_from(child.id()) {
        let _ = process::send_process_group_signal(group, signal);
        if signal != Signal::Kill {
            std::thread::sleep(Duration::from_millis(100));
            let _ = process::send_process_group_signal(group, Signal::Kill);
        }
    } else {
        // Child::id is a positive kernel PID on supported platforms. Retain a
        // final direct-child teardown if a future platform cannot represent it.
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[cfg(test)]
#[path = "preparation/tests.rs"]
mod tests;
