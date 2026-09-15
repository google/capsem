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

mod against_the_service {
    use super::super::*;
    use crate::client::tests::fake_service::FakeService;
    use crate::container_image::{ImageArgs, Workload};
    use crate::{Cli, Commands, SessionCommands};
    use clap::Parser;
    use serde_json::json;

    fn created(id: &str) -> serde_json::Value {
        json!({"id": id, "name": id, "profile_id": "code", "status": "Running", "available_actions": []})
    }

    fn args(argv: &[&str]) -> CreateArgs {
        let argv = ["capsem", "create"].iter().chain(argv).copied();
        match Cli::parse_from(argv).command.unwrap() {
            Commands::Session(SessionCommands::Create(args)) => args,
            _ => panic!("expected Create"),
        }
    }

    #[tokio::test]
    async fn create_asks_for_what_was_given_and_the_profile_for_the_rest() {
        let service = FakeService::start();
        service.route("POST", "/vms/create", 200, created("vm-1"));

        create(&service.client, &args(&["-e", "A=1", "--network", "team"]))
            .await
            .unwrap();
        create(&service.client, &args(&["-n", "keep", "--ram", "2", "--cpu", "3"]))
            .await
            .unwrap();
        create(&service.client, &args(&["--from", "keep"])).await.unwrap();

        let bodies: Vec<_> = service
            .find("POST", "/vms/create")
            .iter()
            .map(|request| request.json())
            .collect();
        assert_eq!(
            bodies[0],
            json!({"name": null, "profile_id": "code", "persistent": false, "env": {"A": "1"}, "networks": ["team"]})
        );
        assert_eq!(
            bodies[1],
            json!({"name": "keep", "profile_id": "code", "ram_mb": 2048, "cpus": 3, "persistent": true})
        );
        assert_eq!(bodies[2]["persistent"], true, "a clone keeps its source's state");
        assert!(create(&service.client, &args(&["--profile", "../x"])).await.is_err());
        assert_eq!(
            service.find("POST", "/vms/create").len(),
            3,
            "an invalid profile never reaches the service"
        );
    }

    #[tokio::test]
    async fn an_image_that_cannot_start_does_not_leave_its_vm_behind() {
        let service = FakeService::start();
        service
            .route(
                "GET",
                "/vms/vm-9/container",
                200,
                json!({"state": "failed", "image": "docker://redis:7", "error": "pull docker://redis:7: no route"}),
            )
            .once("DELETE", "/vms/vm-9/delete", 200, json!({"success": true}))
            .route("DELETE", "/vms/vm-9/delete", 500, json!({"error": "delete stuck"}));
        let image = ImageArgs {
            image: vec!["docker://redis:7".into()],
            ..ImageArgs::default()
        };
        let workload = Workload::of(&image, &[]).unwrap().unwrap();
        let vm: ProvisionResponse = serde_json::from_value(created("vm-9")).unwrap();

        let error = start_image(&service.client, &vm, &workload).await.unwrap_err();
        assert!(format!("{error:#}").contains("no route"), "{error:#}");
        assert_eq!(service.find("DELETE", "/vms/vm-9/delete").len(), 1);

        let error = start_image(&service.client, &vm, &workload).await.unwrap_err();
        let reported = format!("{error:#}");
        assert!(
            reported.contains("no route") && reported.contains("delete stuck"),
            "{reported}"
        );
    }

    #[tokio::test]
    async fn create_image_sends_the_container_and_publishes_once_launched() {
        let service = FakeService::start();
        service
            .route("POST", "/vms/create", 200, created("vm-3"))
            .once(
                "GET",
                "/vms/vm-3/container",
                200,
                json!({"state": "pulling", "image": "docker://redis:7"}),
            )
            .route(
                "GET",
                "/vms/vm-3/container",
                200,
                json!({"state": "starting", "image": "docker://redis:7", "digest": "sha256:ab"}),
            )
            .route(
                "POST",
                "/vms/vm-3/exposures",
                200,
                json!({"id": "4100", "host_port": 4100, "guest_port": 6379, "target": "container"}),
            );
        create(
            &service.client,
            &args(&[
                "-e",
                "A=1",
                "-p",
                "0:6379",
                "--image",
                "docker://redis:7",
                "redis-server",
            ]),
        )
        .await
        .unwrap();

        let body = service.find("POST", "/vms/create")[0].json();
        assert_eq!(
            body["container"],
            json!({"image": "docker://redis:7", "args": ["redis-server"], "env": {"A": "1"}, "attach": false})
        );
        assert!(
            body.get("env").is_none(),
            "with an image the environment is the container's"
        );
        assert_eq!(service.find("GET", "/vms/vm-3/container").len(), 2);
        assert_eq!(
            service.find("POST", "/vms/vm-3/exposures")[0].json(),
            json!({"guest_port": 6379, "host_port": 0, "target": "container"})
        );
        assert!(service.find("DELETE", "/vms/vm-3/delete").is_empty());
    }
}
