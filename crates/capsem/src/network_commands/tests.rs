use super::*;
use crate::{Cli, Commands, SessionCommands};
use clap::Parser;

#[test]
fn parse_create_with_repeated_networks() {
    let cli = Cli::parse_from(["capsem", "create", "--network", "team", "--network", "ci"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create(crate::create_command::CreateArgs { network, .. })) => {
            assert_eq!(network, vec!["team", "ci"]);
        }
        _ => panic!("expected Create"),
    }
}

#[test]
fn parse_network_commands() {
    let cli = Cli::parse_from(["capsem", "network", "connect", "my-vm", "team"]);
    match cli.command.unwrap() {
        Commands::Network(NetworkCommands::Connect { session, network }) => {
            assert_eq!(session, "my-vm");
            assert_eq!(network, "team");
        }
        _ => panic!("expected network connect"),
    }
    let cli = Cli::parse_from(["capsem", "network", "list"]);
    assert!(matches!(cli.command.unwrap(), Commands::Network(NetworkCommands::List)));
    let cli = Cli::parse_from(["capsem", "network", "create", "team"]);
    assert!(matches!(cli.command.unwrap(), Commands::Network(NetworkCommands::Create { name }) if name == "team"));
}

#[test]
fn parse_network_logs_with_follow_and_filters() {
    let cli = Cli::parse_from([
        "capsem",
        "network",
        "logs",
        "team",
        "-f",
        "--limit",
        "10",
        "--type",
        "network.connect",
    ]);
    match cli.command.unwrap() {
        Commands::Network(NetworkCommands::Logs {
            network,
            follow,
            limit,
            event_type,
            ..
        }) => {
            assert_eq!(network, "team");
            assert!(follow);
            assert_eq!(limit, 10);
            assert_eq!(event_type.as_deref(), Some("network.connect"));
        }
        _ => panic!("expected network logs"),
    }
}

mod against_the_service {
    use super::super::*;
    use crate::client::tests::fake_service::FakeService;
    use serde_json::json;

    fn team(members: serde_json::Value) -> serde_json::Value {
        json!({"id": "net-1", "name": "team", "subnet": "10.128.4.0/24", "created_unix_ms": 1, "members": members})
    }

    #[test]
    fn a_network_carries_the_subnet_its_members_are_addressed_from() {
        let network: NetworkInfo = serde_json::from_value(team(json!([]))).unwrap();
        assert_eq!(network.subnet, "10.128.4.0/24");
        let missing = json!({"id": "net-1", "name": "team", "created_unix_ms": 1, "members": []});
        assert!(
            serde_json::from_value::<NetworkInfo>(missing).is_err(),
            "a network without its subnet is not a network the service sends"
        );
    }

    fn scripted() -> FakeService {
        let service = FakeService::start();
        service
            .route("GET", "/networks/net-1/logs", 200, json!({"events": [], "cursor": "c"}))
            .route(
                "GET",
                "/networks/net-1",
                200,
                team(json!([
                    {"vm_id": "vm-1", "address": "10.0.0.2", "state": "linked", "updated_unix_ms": 2}
                ])),
            )
            .route("GET", "/networks", 200, json!({"networks": [team(json!([]))]}))
            .route("POST", "/networks", 200, team(json!([])))
            .route("DELETE", "/networks/net-1/members/vm-1", 200, team(json!([])))
            .route("DELETE", "/networks/net-1", 200, json!({"success": true}))
            .route("PUT", "/networks/net-1/members/vm-1", 200, team(json!([])))
            .route(
                "GET",
                "/vms/list",
                200,
                json!({"sandboxes": [
                    {"id": "vm-1", "name": "dev", "pid": 1, "status": "Running"}
                ]}),
            );
        service
    }

    #[tokio::test]
    async fn names_resolve_to_ids_before_any_route_that_takes_one() {
        let service = scripted();
        for command in [
            NetworkCommands::List,
            NetworkCommands::Create { name: "team".into() },
            NetworkCommands::Inspect { network: "team".into() },
            NetworkCommands::Connect {
                session: "dev".into(),
                network: "team".into(),
            },
            NetworkCommands::Disconnect {
                session: "vm-1".into(),
                network: "net-1".into(),
            },
            NetworkCommands::Delete { network: "team".into() },
        ] {
            run(&service.client, &command).await.unwrap();
        }
        assert_eq!(
            service.calls(),
            [
                "GET /networks",
                "POST /networks",
                "GET /networks",
                "GET /networks/net-1",
                "GET /vms/list",
                "GET /networks",
                "PUT /networks/net-1/members/vm-1",
                "GET /vms/list",
                "GET /networks",
                "DELETE /networks/net-1/members/vm-1",
                "GET /networks",
                "DELETE /networks/net-1",
            ]
        );
        assert_eq!(service.find("POST", "/networks")[0].json(), json!({"name": "team"}));
    }

    #[tokio::test]
    async fn unknown_names_and_invalid_ids_never_reach_a_route() {
        let service = scripted();
        let unknown = run(
            &service.client,
            &NetworkCommands::Inspect {
                network: "other".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(
            unknown.to_string().contains("unknown network name or id: other"),
            "{unknown}"
        );
        let session = NetworkCommands::Connect {
            session: "ghost".into(),
            network: "team".into(),
        };
        let unknown = run(&service.client, &session).await.unwrap_err();
        assert!(
            unknown.to_string().contains("unknown session name or id: ghost"),
            "{unknown}"
        );
        assert!(run(&service.client, &NetworkCommands::Create { name: "../x".into() })
            .await
            .is_err());
        assert_eq!(service.calls(), ["GET /networks", "GET /vms/list"]);
    }

    #[tokio::test]
    async fn logs_page_through_every_cursor_with_the_filters() {
        let service = FakeService::start();
        let event = json!({
            "sequence": 1, "event_id": "e1", "timestamp_unix_ms": 1, "event_type": "network.connect",
            "connection_id": "c1", "event": {"decision": "allow"},
        });
        service
            .route("GET", "/networks", 200, json!({"networks": [team(json!([]))]}))
            .once(
                "GET",
                "/networks/net-1/logs",
                200,
                json!({"events": [event], "cursor": "p1", "next_cursor": "p2"}),
            )
            .once(
                "GET",
                "/networks/net-1/logs",
                200,
                json!({"events": [event], "cursor": "p2"}),
            );
        let command = NetworkCommands::Logs {
            network: "team".into(),
            follow: false,
            limit: 1,
            vm: Some("vm 1".into()),
            event_type: Some("network.connect".into()),
            decision: None,
        };
        run(&service.client, &command).await.unwrap();
        assert_eq!(
            service.calls()[1..],
            [
                "GET /networks/net-1/logs?limit=1&vm=vm%201&type=network.connect",
                "GET /networks/net-1/logs?limit=1&cursor=p1&vm=vm%201&type=network.connect",
            ]
        );
    }
}
