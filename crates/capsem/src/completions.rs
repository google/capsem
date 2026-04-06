use std::io;
use clap::CommandFactory;
use clap_complete::{Shell, generate};

use crate::Cli;

/// Generate shell completions for the given shell and write to stdout.
pub fn generate_completions(shell: Shell) {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    generate(shell, &mut cmd, name, &mut io::stdout());
}
