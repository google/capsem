//! `capsem network`: named networks over the service API.
use crate::client::{self, ApiResponse, CreateNetworkRequest, NetworkInfo, NetworkListResponse, UdsClient};
use crate::route_ids::{resolve_network_route_id, resolve_session_route_id};
use crate::session_display::{print_network_info, print_network_list};
use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum NetworkCommands {
    /// List named networks
    List,
    /// Create a named network (a DNS label)
    Create {
        /// Network name
        name: String,
    },
    /// Show a network's id and members
    Inspect {
        /// Network name or id
        network: String,
    },
    /// Retire an empty network
    Delete {
        /// Network name or id
        network: String,
    },
    /// Connect a session to a network
    Connect {
        /// Session name or id
        session: String,
        /// Network name or id
        network: String,
    },
    /// Disconnect a session from a network
    Disconnect {
        /// Session name or id
        session: String,
        /// Network name or id
        network: String,
    },
}

pub(crate) async fn run(client: &UdsClient, command: &NetworkCommands) -> Result<()> {
    match command {
        NetworkCommands::List => {
            let resp: ApiResponse<NetworkListResponse> = client.get("/networks").await?;
            let list = resp.into_result()?;
            print_network_list(&list.networks);
        }
        NetworkCommands::Create { name } => {
            client::validate_id(name)?;
            let resp: ApiResponse<NetworkInfo> = client
                .post("/networks", &CreateNetworkRequest { name: name.clone() })
                .await?;
            let network = resp.into_result()?;
            println!("[*] Network \"{}\" created ({})", network.name, network.id);
        }
        NetworkCommands::Inspect { network } => {
            let network_id = resolve_network_route_id(client, network).await?;
            let resp: ApiResponse<NetworkInfo> = client.get(&format!("/networks/{network_id}")).await?;
            print_network_info(&resp.into_result()?);
        }
        NetworkCommands::Delete { network } => {
            let network_id = resolve_network_route_id(client, network).await?;
            let resp: ApiResponse<serde_json::Value> = client.delete(&format!("/networks/{network_id}")).await?;
            resp.into_result()?;
            println!("Network deleted.");
        }
        NetworkCommands::Connect { session, network } => {
            client::validate_id(session)?;
            let session_id = resolve_session_route_id(client, session).await?;
            let network_id = resolve_network_route_id(client, network).await?;
            let resp: ApiResponse<NetworkInfo> = client
                .put(&format!("/networks/{network_id}/members/{session_id}"))
                .await?;
            let joined = resp.into_result()?;
            println!("[*] Session \"{}\" connected to \"{}\"", session, joined.name);
        }
        NetworkCommands::Disconnect { session, network } => {
            client::validate_id(session)?;
            let session_id = resolve_session_route_id(client, session).await?;
            let network_id = resolve_network_route_id(client, network).await?;
            let resp: ApiResponse<NetworkInfo> = client
                .delete(&format!("/networks/{network_id}/members/{session_id}"))
                .await?;
            let left = resp.into_result()?;
            println!("[*] Session \"{}\" disconnected from \"{}\"", session, left.name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
