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
