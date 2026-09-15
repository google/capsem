//! `capsem network`: named networks over the service API.
use crate::client::{
    self, ApiResponse, CreateNetworkRequest, NetworkInfo, NetworkListResponse, NetworkLogsFilter, UdsClient,
};
use crate::route_ids::{resolve_network_route_id, resolve_session_route_id};
use crate::session_display::{print_network_info, print_network_list, print_network_log_event};
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
    /// Show a network's audit log, oldest first
    Logs {
        /// Network name or id
        network: String,
        /// Keep printing new events until interrupted
        #[arg(short = 'f', long)]
        follow: bool,
        /// Events per page (1-1000)
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Only events involving this session id
        #[arg(long)]
        vm: Option<String>,
        /// Only events of this type, e.g. network.connect
        #[arg(long = "type")]
        event_type: Option<String>,
        /// Only events with this effective decision: allow, ask or block
        #[arg(long)]
        decision: Option<String>,
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
        NetworkCommands::Logs {
            network,
            follow,
            limit,
            vm,
            event_type,
            decision,
        } => {
            let network_id = resolve_network_route_id(client, network).await?;
            let filter = NetworkLogsFilter {
                vm: vm.clone(),
                event_type: event_type.clone(),
                decision: decision.clone(),
            };
            follow_network_logs(client, &network_id, *limit, &filter, *follow).await?;
        }
    }
    Ok(())
}

/// Print every page from the start; with `follow`, keep polling the last
/// cursor once a second until Ctrl-C.
async fn follow_network_logs(
    client: &UdsClient,
    network_id: &str,
    limit: usize,
    filter: &NetworkLogsFilter,
    follow: bool,
) -> Result<()> {
    let mut cursor: Option<String> = None;
    loop {
        let page = client
            .network_logs(network_id, cursor.as_deref(), limit, filter)
            .await?;
        for event in &page.events {
            print_network_log_event(event);
        }
        cursor = Some(page.cursor);
        if page.next_cursor.is_some() {
            continue;
        }
        if !follow {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
