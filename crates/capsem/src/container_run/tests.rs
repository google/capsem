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

mod against_the_service {
    use super::super::*;
    use crate::client::tests::fake_service::FakeService;
    use crate::{Cli, Commands, SessionCommands};
    use clap::Parser;
    use serde_json::json;

    fn args(argv: &[&str]) -> RunArgs {
        let argv = ["capsem", "run"].iter().chain(argv).copied();
        match Cli::parse_from(argv).command.unwrap() {
            Commands::Session(SessionCommands::Run(args)) => args,
            _ => panic!("expected Run"),
        }
    }

    #[tokio::test]
    async fn a_shell_run_returns_the_guest_exit_code_and_sends_only_what_was_given() {
        let service = FakeService::start();
        service.route(
            "POST",
            "/run",
            200,
            json!({"stdout": "out\n", "stderr": "err\n", "exit_code": 7, "truncated": true}),
        );
        let code = run(
            &service.client,
            &args(&["false", "--timeout", "5", "--ram", "1", "-e", "K=V"]),
        )
        .await
        .unwrap();
        assert_eq!(code, 7);
        assert_eq!(
            service.find("POST", "/run")[0].json(),
            json!({"command": "false", "profile_id": "code", "timeout_secs": 5, "ram_mb": 1024, "env": {"K": "V"}})
        );
    }

    #[tokio::test]
    async fn a_run_without_work_or_with_image_only_flags_is_refused_locally() {
        let service = FakeService::start();
        for (argv, expected) in [
            (vec![], "run needs a shell command, or --image"),
            (vec!["true", "--network", "team"], "--network needs --image"),
            (vec!["true", "-p", "0:80"], "need --image"),
            (vec!["true", "--profile", "../x"], ""),
        ] {
            let error = run(&service.client, &args(&argv)).await.unwrap_err();
            assert!(format!("{error:#}").contains(expected), "{argv:?}: {error:#}");
        }
        assert!(service.calls().is_empty(), "{:?}", service.calls());
    }

    #[tokio::test]
    async fn a_service_error_is_the_run_error() {
        let service = FakeService::start();
        service.route("POST", "/run", 503, json!({"error": "profile assets missing"}));
        let error = run(&service.client, &args(&["true"])).await.unwrap_err();
        assert!(format!("{error:#}").contains("profile assets missing"), "{error:#}");
    }
}
