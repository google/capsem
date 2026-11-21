//! Exact request review and explicit decisions through the gateway.
use crate::client::GatewayClient;
use anyhow::Result;
use capsem_api::requests::{RequestDecision, RequestResolveRequest};
use capsem_sdk::operations::ListRequestsParams;
use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub(crate) enum RequestCommands {
    /// List a page of pending requests as JSON
    List {
        /// Include decided and expired requests
        #[arg(long)]
        all: bool,
        /// Continue from a previous page's next_cursor
        #[arg(long, value_parser = clap::value_parser!(i64).range(0..))]
        after: Option<i64>,
        /// Maximum number of requests per page
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..=i64::from(capsem_proto::APPROVAL_PENDING_TOTAL as u16)))]
        limit: Option<u16>,
    },
    /// Show the frozen review inputs and application state as JSON
    Info {
        /// Request UUID from the list or proposal receipt
        id: String,
    },
    /// Submit a decision for the reviewed observation and print its JSON receipt
    Resolve {
        /// Request UUID from the review
        id: String,
        /// Operators approve or deny; the scoped requester can cancel
        decision: Decision,
        /// Exact observed_revision from request info or the original proposal receipt
        #[arg(long)]
        observed_revision: String,
        /// Reason recorded with the decision
        #[arg(long)]
        reason: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Decision {
    Approve,
    Deny,
    Cancel,
}

pub(crate) async fn run(client: &GatewayClient, command: &RequestCommands) -> Result<()> {
    let output = match command {
        RequestCommands::List { all, after, limit } => {
            let page = client
                .list_requests(&ListRequestsParams {
                    after: *after,
                    limit: limit.map(u32::from),
                    pending_only: Some(!all),
                })
                .await?;
            serde_json::to_string_pretty(&page)?
        }
        RequestCommands::Info { id } => serde_json::to_string_pretty(&client.get_request_info(id).await?)?,
        RequestCommands::Resolve {
            id,
            decision,
            observed_revision,
            reason,
        } => {
            let decision = match decision {
                Decision::Approve => RequestDecision::Approve,
                Decision::Deny => RequestDecision::Deny,
                Decision::Cancel => RequestDecision::Cancel,
            };
            let receipt = client
                .resolve_request(
                    id,
                    RequestResolveRequest {
                        observed_revision: observed_revision.clone(),
                        decision,
                        reason: reason.clone(),
                    },
                )
                .await?;
            serde_json::to_string_pretty(&receipt)?
        }
    };
    println!("{output}");
    Ok(())
}
