//! Named networks on the wire, and the calls that reach them.
use super::{ApiResponse, Result, UdsClient};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct CreateNetworkRequest {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkMemberInfo {
    pub vm_id: String,
    pub address: String,
    pub state: String,
    pub updated_unix_ms: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkInfo {
    pub id: String,
    pub name: String,
    pub created_unix_ms: i64,
    #[serde(default)]
    pub members: Vec<NetworkMemberInfo>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NetworkListResponse {
    pub networks: Vec<NetworkInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkLogEvent {
    pub sequence: i64,
    pub event_id: String,
    pub timestamp_unix_ms: i64,
    pub event_type: String,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub event: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NetworkLogsResponse {
    pub events: Vec<NetworkLogEvent>,
    pub cursor: String,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// The filters `capsem network logs` forwards; every one is optional.
#[derive(Debug, Default, Clone)]
pub struct NetworkLogsFilter {
    pub vm: Option<String>,
    pub event_type: Option<String>,
    pub decision: Option<String>,
}

impl UdsClient {
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
