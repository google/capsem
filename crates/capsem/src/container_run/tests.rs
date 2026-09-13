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
            assert_eq!(command.as_deref(), Some("echo hello"));
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
            assert_eq!(command.as_deref(), Some("ls -la"));
            assert_eq!(profile, "code");
            assert_eq!(timeout, Some(120));
            assert!(env.is_empty());
        }
        _ => panic!("expected Run"),
    }
}

#[test]
fn run_image_takes_its_command_resources_and_no_name() {
    let cli = Cli::parse_from([
        "capsem",
        "run",
        "--ram",
        "2",
        "--cpu",
        "1",
        "--network",
        "team",
        "--image",
        "docker://redis:7-alpine",
        "redis-server",
        "--save",
        "",
    ]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Run(RunArgs {
            command,
            args,
            image,
            ram,
            cpu,
            network,
            ..
        })) => {
            assert_eq!(image.image.as_deref(), Some("docker://redis:7-alpine"));
            assert_eq!(command.as_deref(), Some("redis-server"));
            assert_eq!(args, ["--save", ""]);
            assert_eq!((ram, cpu), (Some(2), Some(1)));
            assert_eq!(network, ["team"]);
        }
        _ => panic!("expected Run"),
    }
    assert!(
        Cli::try_parse_from(["capsem", "run", "-n", "keep", "--image", "docker://redis"]).is_err(),
        "a run is one-shot; a named VM is a create"
    );
}

#[test]
fn a_positional_image_is_a_shell_command_and_container_flags_need_an_image() {
    let cli = Cli::parse_from(["capsem", "run", "docker://redis"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Run(RunArgs { command, image, .. })) => {
            assert_eq!(command.as_deref(), Some("docker://redis"));
            assert!(image.image.is_none());
        }
        _ => panic!("expected Run"),
    }
    for argv in [
        vec!["capsem", "run", "-p", "0:80", "true"],
        vec!["capsem", "run", "--network", "team", "true"],
        vec!["capsem", "run", "true", "extra"],
    ] {
        assert!(Cli::try_parse_from(&argv).is_err(), "accepted {argv:?}");
    }
}

#[test]
fn ram_is_given_in_gigabytes_and_unset_stays_unset() {
    assert_eq!(ram_mb(Some(4)), Some(4096));
    assert_eq!(ram_mb(None), None);
    assert_eq!(ram_mb(Some(u64::MAX)), Some(u64::MAX));
}
