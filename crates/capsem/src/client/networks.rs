//! Named networks on the wire, and the calls that reach them.
use super::{ApiResponse, GatewayClient, Result};
use serde::Deserialize;

pub use capsem_api::{CreateNetworkRequest, NetworkInfo, NetworkListResponse, NetworkLogEvent, NetworkLogsResponse};

/// The filters `capsem network logs` forwards; every one is optional.
#[derive(Debug, Default, Clone)]
pub struct NetworkLogsFilter {
    pub vm: Option<String>,
    pub event_type: Option<String>,
    pub decision: Option<String>,
}

impl GatewayClient {
    /// One page of a network's audit log; `cursor` continues a previous page.
    pub async fn network_logs(
        &self,
        network_id: &str,
        cursor: Option<&str>,
        limit: usize,
        filter: &NetworkLogsFilter,
    ) -> Result<NetworkLogsResponse> {
        let mut query = vec![format!("limit={limit}")];
        for (key, value) in [
            ("cursor", cursor.map(str::to_string)),
            ("vm", filter.vm.clone()),
            ("type", filter.event_type.clone()),
            ("decision", filter.decision.clone()),
        ] {
            if let Some(value) = value {
                query.push(format!("{key}={}", percent_encode(&value)));
            }
        }
        let response: ApiResponse<NetworkLogsResponse> = self
            .get(&format!("/networks/{network_id}/logs?{}", query.join("&")))
            .await?;
        response.into_result()
    }

    pub async fn put<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        self.request::<(), R>("PUT", path, None).await
    }

    /// Resolve a network's id from either its id or its name.
    pub async fn listed_network_id(&self, typed: &str) -> Result<Option<String>> {
        let response: ApiResponse<NetworkListResponse> = self.get("/networks").await?;
        let list = response.into_result()?;
        Ok(list
            .networks
            .iter()
            .find(|network| network.id == typed || network.name == typed)
            .map(|network| network.id.clone()))
    }
}

/// Enough escaping for cursor, id and name values in a query string.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
