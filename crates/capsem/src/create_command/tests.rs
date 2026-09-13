use super::*;
use crate::container_image::Workload;
use crate::{Cli, Commands, SessionCommands};
use clap::Parser;

#[test]
fn create_image_takes_everything_after_the_image_as_its_command() {
    let cli = Cli::parse_from([
        "capsem",
        "create",
        "-n",
        "cache",
        "-p",
        "0:6379",
        "--image",
        "docker://redis:7-alpine",
        "redis-server",
        "--save",
        "",
        "-p",
        "7",
    ]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create(CreateArgs { name, from, image, .. })) => {
            assert_eq!(name.as_deref(), Some("cache"));
            assert_eq!(from, None, "--image is an OCI image, not a clone source");
            assert_eq!(image.publish.len(), 1, "options go before the image");
            let workload = Workload::of(&image, &[]).unwrap().unwrap();
            assert_eq!(workload.reference, "docker://redis:7-alpine");
            assert_eq!(workload.args, ["redis-server", "--save", "", "-p", "7"]);
        }
        _ => panic!("expected Create with --image"),
    }
}

#[test]
fn image_only_flags_and_clone_sources_do_not_mix_with_plain_create() {
    for argv in [
        vec!["capsem", "create", "--from", "base", "--image", "docker://redis"],
        vec!["capsem", "create", "redis-server"],
        vec!["capsem", "create", "--image"],
    ] {
        assert!(Cli::try_parse_from(&argv).is_err(), "accepted {argv:?}");
    }
    for argv in [
        ["capsem", "create", "-p", "0:80"],
        ["capsem", "create", "--registry-user", "me"],
    ] {
        let Commands::Session(SessionCommands::Create(args)) = Cli::parse_from(argv).command.unwrap() else {
            panic!("expected Create")
        };
        assert!(Workload::of(&args.image, &args.env).is_err(), "accepted {argv:?}");
    }
}
