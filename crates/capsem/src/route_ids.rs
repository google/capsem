//! Sessions and networks are addressed by name or id; the service routes
//! take ids. Both resolvers read the listing the service already serves.
use crate::client::{self, UdsClient};

pub(crate) async fn resolve_network_route_id(client: &UdsClient, typed: &str) -> anyhow::Result<String> {
    client::validate_id(typed)?;
    client
        .listed_network_id(typed)
        .await?
        .ok_or_else(|| anyhow::anyhow!("unknown network name or id: {typed}"))
}

pub(crate) async fn resolve_session_route_id(client: &UdsClient, typed: &str) -> anyhow::Result<String> {
    client
        .listed_session_id(typed)
        .await?
        .ok_or_else(|| anyhow::anyhow!("unknown session name or id: {typed}"))
}
