use super::*;
use crate::{Cli, Commands, SessionCommands};
use clap::Parser;

#[test]
fn cli_run_accepts_profile() {
    let cli = Cli::parse_from(["capsem", "run", "echo ok", "--profile", "co-work"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Run(RunArgs { profile, .. })) => {
            assert_eq!(profile, "co-work");
        }
        _ => panic!("expected Run"),
    }
}

#[test]
fn parse_run() {
    let cli = Cli::parse_from(["capsem", "run", "echo hello"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Run(RunArgs {
            command,
            profile,
            timeout,
            env,
            ..
        })) => {
            assert_eq!(command, "echo hello");
            assert_eq!(profile, "code");
            assert_eq!(timeout, None);
            assert!(env.is_empty());
        }
        _ => panic!("expected Run"),
    }
}

#[test]
fn parse_run_with_timeout() {
    let cli = Cli::parse_from(["capsem", "run", "--timeout", "120", "ls -la"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Run(RunArgs {
            command,
            profile,
            timeout,
            env,
            ..
        })) => {
            assert_eq!(command, "ls -la");
            assert_eq!(profile, "code");
            assert_eq!(timeout, Some(120));
            assert!(env.is_empty());
        }
        _ => panic!("expected Run"),
    }
}

#[test]
fn parse_image_run_with_name_and_command_arguments() {
    assert!(Cli::try_parse_from([
        "capsem",
        "run",
        "--name",
        "redis-proof",
        "docker://redis:7-alpine",
        "redis-server",
        "--save",
        ""
    ])
    .is_ok());
}
