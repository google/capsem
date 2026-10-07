//! What `capsem-pty-agent` does with its command line.
//!
//! The agent is started once by init, with no arguments, and owns the VM's
//! terminal and control vsock links. Anything else that runs it -- an AI agent
//! exploring the guest ran `/run/capsem-pty-agent --help` -- must not get as
//! far as connecting: a second instance displaced the first's links and the
//! whole VM hung (#198). Arguments are answered and refused here, before any
//! vsock work; a second argument-free instance is refused by the singleton
//! lock in `main`.

/// Where the running agent holds its singleton lock. `/run` is tmpfs and only
/// root can write it, so it disappears with the VM and no user can pre-claim it.
pub(crate) const LOCK_PATH: &str = "/run/capsem-pty-agent.lock";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CliAction {
    /// No arguments: the init-started agent.
    Run,
    /// Print this to stdout or stderr and exit with the code.
    Exit { code: i32, message: String },
}

pub(crate) fn cli_action(args: &[String]) -> CliAction {
    let usage = format!(
        "capsem-pty-agent {}\n\
         The Capsem guest agent. It is started once by the VM's init and owns the\n\
         terminal and control links to the host; running it by hand does nothing.\n\
         \n\
         Usage: capsem-pty-agent [--help | --version]",
        env!("CARGO_PKG_VERSION")
    );
    match args {
        [] => CliAction::Run,
        [flag] if flag == "--help" || flag == "-h" => CliAction::Exit {
            code: 0,
            message: usage,
        },
        [flag] if flag == "--version" || flag == "-V" => CliAction::Exit {
            code: 0,
            message: format!("capsem-pty-agent {}", env!("CARGO_PKG_VERSION")),
        },
        _ => CliAction::Exit {
            code: 2,
            message: usage,
        },
    }
}

#[cfg(test)]
mod tests;
