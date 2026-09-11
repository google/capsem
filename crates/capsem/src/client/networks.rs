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

impl UdsClient {
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
