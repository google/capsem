use super::*;
use crate::{Cli, Commands, SessionCommands};
use clap::Parser;

#[test]
fn parse_create_with_repeated_networks() {
    let cli = Cli::parse_from(["capsem", "create", "--network", "team", "--network", "ci"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { network, .. }) => {
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
