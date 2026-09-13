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
fn run_image_takes_a_shell_style_command_after_the_image() {
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
        "docker://alpine:3",
        "sh",
        "-c",
        "echo hi",
    ]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Run(RunArgs {
            command,
            image,
            ram,
            cpu,
            network,
            ..
        })) => {
            assert_eq!(command, None);
            let workload = crate::container_image::Workload::of(&image, &[]).unwrap().unwrap();
            assert_eq!(workload.reference, "docker://alpine:3");
            assert_eq!(workload.args, ["sh", "-c", "echo hi"]);
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
    let cli = Cli::parse_from(["capsem", "run", "docker://redis", "--timeout", "5"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Run(RunArgs {
            command,
            image,
            timeout,
            ..
        })) => {
            assert_eq!(command.as_deref(), Some("docker://redis"));
            assert!(image.image.is_empty());
            assert_eq!(timeout, Some(5), "flags after a plain command still parse");
        }
        _ => panic!("expected Run"),
    }
    for argv in [
        vec!["capsem", "run", "true", "extra"],
        vec!["capsem", "run", "true", "--image", "docker://redis"],
    ] {
        assert!(Cli::try_parse_from(&argv).is_err(), "accepted {argv:?}");
    }
    let cli = Cli::parse_from(["capsem", "run", "-p", "0:80", "true"]);
    let Commands::Session(SessionCommands::Run(args)) = cli.command.unwrap() else {
        panic!("expected Run")
    };
    let refused = crate::container_image::Workload::of(&args.image, &args.env)
        .err()
        .unwrap();
    assert!(format!("{refused}").contains("need --image"), "{refused}");
}

#[test]
fn ram_is_given_in_gigabytes_and_unset_stays_unset() {
    assert_eq!(ram_mb(Some(4)), Some(4096));
    assert_eq!(ram_mb(None), None);
    assert_eq!(ram_mb(Some(u64::MAX)), Some(u64::MAX));
}
