use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use utoipa::{IntoParams, ToSchema};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct CreateNetworkRequest {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMemberState {
    Declared,
    Attaching,
    Ready,
    Failed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct NetworkMemberInfo {
    pub vm_id: String,
    #[schema(value_type = String, format = "ipv4")]
    pub address: Ipv4Addr,
    pub state: NetworkMemberState,
    pub updated_unix_ms: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct NetworkInfo {
    pub id: String,
    pub name: String,
    /// CIDR allocated to this network.
    pub subnet: String,
    pub created_unix_ms: i64,
    pub members: Vec<NetworkMemberInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct NetworkListResponse {
    pub networks: Vec<NetworkInfo>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq, IntoParams)]
pub struct NetworkLogsQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub vm: Option<String>,
    pub connection: Option<String>,
    #[serde(rename = "type")]
    #[param(rename = "type")]
    pub event_type: Option<String>,
    pub decision: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ToSchema)]
pub struct NetworkLogEvent {
    pub sequence: i64,
    pub event_id: String,
    pub timestamp_unix_ms: i64,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    pub event: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ToSchema)]
pub struct NetworkLogsResponse {
    pub events: Vec<NetworkLogEvent>,
    pub cursor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
