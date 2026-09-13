use super::*;
use crate::{Cli, Commands, SessionCommands};
use clap::Parser;

#[test]
fn create_image_names_the_workload_and_takes_its_command() {
    let cli = Cli::parse_from([
        "capsem",
        "create",
        "-n",
        "cache",
        "--image",
        "docker://redis:7-alpine",
        "-p",
        "0:6379",
        "redis-server",
        "--save",
        "",
    ]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create(CreateArgs {
            name,
            from,
            image,
            args,
            ..
        })) => {
            assert_eq!(name.as_deref(), Some("cache"));
            assert_eq!(from, None, "--image is an OCI image, not a clone source");
            assert_eq!(image.image.as_deref(), Some("docker://redis:7-alpine"));
            assert_eq!(image.publish.len(), 1);
            assert_eq!(args, ["redis-server", "--save", ""]);
        }
        _ => panic!("expected Create with --image"),
    }
}

#[test]
fn image_only_flags_and_clone_sources_do_not_mix_with_plain_create() {
    for argv in [
        vec!["capsem", "create", "-p", "0:80"],
        vec!["capsem", "create", "--registry-user", "me"],
        vec!["capsem", "create", "--from", "base", "--image", "docker://redis"],
        vec!["capsem", "create", "redis-server"],
    ] {
        assert!(Cli::try_parse_from(&argv).is_err(), "accepted {argv:?}");
    }
}
