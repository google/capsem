use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::Deserialize;
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[allow(dead_code)]
pub struct StatusResponse {
    pub service: String,
    pub vm_count: u32,
    pub vms: Vec<VmSummary>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[allow(dead_code)]
pub struct VmSummary {
    pub id: String,
    pub name: Option<String>,
    pub status: String,
    pub persistent: bool,
}

pub struct GatewayClient {
    port: u16,
    token: String,
    client: reqwest::Client,
}

impl GatewayClient {
    /// Discover gateway connection info from well-known file paths.
    /// If `port_override` is set, use that instead of reading the file.
    pub async fn discover(port_override: Option<u16>) -> Result<Self> {
        let home = std::env::var("HOME").context("HOME not set")?;
        let run_dir = PathBuf::from(home).join(".capsem/run");

        let port = match port_override {
            Some(p) => p,
            None => {
                let port_str = tokio::fs::read_to_string(run_dir.join("gateway.port"))
                    .await
                    .context("failed to read gateway.port")?;
                port_str
                    .trim()
                    .parse::<u16>()
                    .context("invalid port in gateway.port")?
            }
        };

        let token = tokio::fs::read_to_string(run_dir.join("gateway.token"))
            .await
            .context("failed to read gateway.token")?
            .trim()
            .to_string();

        debug!(port, "gateway discovered");

        Ok(Self {
            port,
            token,
            client: reqwest::Client::new(),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn auth_header(&self) -> HeaderValue {
        HeaderValue::from_str(&format!("Bearer {}", self.token))
            .expect("token contains invalid header chars")
    }

    async fn get(&self, path: &str) -> Result<reqwest::Response> {
        let resp = self
            .client
            .get(format!("{}{path}", self.base_url()))
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .await
            .context("gateway request failed")?;

        if !resp.status().is_success() {
            bail!("gateway returned {}", resp.status());
        }
        Ok(resp)
    }

    async fn post(&self, path: &str) -> Result<reqwest::Response> {
        let resp = self
            .client
            .post(format!("{}{path}", self.base_url()))
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .await
            .context("gateway request failed")?;

        if !resp.status().is_success() {
            bail!("gateway returned {}", resp.status());
        }
        Ok(resp)
    }

    async fn delete_req(&self, path: &str) -> Result<reqwest::Response> {
        let resp = self
            .client
            .delete(format!("{}{path}", self.base_url()))
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .await
            .context("gateway request failed")?;

        if !resp.status().is_success() {
            bail!("gateway returned {}", resp.status());
        }
        Ok(resp)
    }

    pub async fn status(&self) -> Result<StatusResponse> {
        let resp = self.get("/status").await?;
        resp.json().await.context("failed to parse status response")
    }

    pub async fn stop_vm(&self, id: &str) -> Result<()> {
        self.post(&format!("/stop/{id}")).await?;
        Ok(())
    }

    pub async fn delete_vm(&self, id: &str) -> Result<()> {
        self.delete_req(&format!("/delete/{id}")).await?;
        Ok(())
    }

    pub async fn suspend_vm(&self, id: &str) -> Result<()> {
        self.post(&format!("/suspend/{id}")).await?;
        Ok(())
    }

    pub async fn resume_vm(&self, id: &str) -> Result<()> {
        self.post(&format!("/resume/{id}")).await?;
        Ok(())
    }

    pub async fn fork_vm(&self, id: &str) -> Result<()> {
        self.post(&format!("/fork/{id}")).await?;
        Ok(())
    }

    /// Provision a temporary (ephemeral) VM. Returns the new VM id.
    pub async fn provision_temp(&self) -> Result<String> {
        let resp = self.post("/provision").await?;
        let body: serde_json::Value = resp.json().await?;
        body["id"]
            .as_str()
            .map(|s| s.to_string())
            .context("provision response missing id")
    }

    /// Provision a named (persistent) VM. Returns the new VM id.
    #[allow(dead_code)]
    pub async fn provision_named(&self, name: &str) -> Result<String> {
        let resp = self
            .client
            .post(format!("{}/provision", self.base_url()))
            .header(AUTHORIZATION, self.auth_header())
            .json(&serde_json::json!({ "name": name }))
            .send()
            .await
            .context("gateway request failed")?;

        if !resp.status().is_success() {
            bail!("gateway returned {}", resp.status());
        }

        let body: serde_json::Value = resp.json().await?;
        body["id"]
            .as_str()
            .map(|s| s.to_string())
            .context("provision response missing id")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_status_response() {
        let json = r#"{
            "service": "running",
            "vm_count": 2,
            "vms": [
                {"id": "abc123", "name": "dev", "status": "running", "persistent": true},
                {"id": "def456", "name": null, "status": "stopped", "persistent": false}
            ]
        }"#;
        let resp: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.service, "running");
        assert_eq!(resp.vm_count, 2);
        assert_eq!(resp.vms.len(), 2);
        assert_eq!(resp.vms[0].name.as_deref(), Some("dev"));
        assert!(resp.vms[0].persistent);
        assert_eq!(resp.vms[1].name, None);
        assert!(!resp.vms[1].persistent);
    }

    #[test]
    fn deserialize_empty_vm_list() {
        let json = r#"{"service": "running", "vm_count": 0, "vms": []}"#;
        let resp: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.vm_count, 0);
        assert!(resp.vms.is_empty());
    }

    #[test]
    fn deserialize_extra_fields_ignored() {
        let json = r#"{
            "service": "running",
            "vm_count": 0,
            "vms": [],
            "extra_field": "should be ignored"
        }"#;
        let resp: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.vm_count, 0);
    }

    #[test]
    fn deserialize_vm_extra_fields_ignored() {
        let json = r#"{
            "id": "abc",
            "name": "test",
            "status": "running",
            "persistent": true,
            "ram_mb": 512,
            "cpus": 4
        }"#;
        let vm: VmSummary = serde_json::from_str(json).unwrap();
        assert_eq!(vm.id, "abc");
    }

    #[test]
    fn base_url_format() {
        let client = GatewayClient {
            port: 19222,
            token: "test-token".into(),
            client: reqwest::Client::new(),
        };
        assert_eq!(client.base_url(), "http://127.0.0.1:19222");
    }

    #[test]
    fn auth_header_format() {
        let client = GatewayClient {
            port: 8080,
            token: "my-secret".into(),
            client: reqwest::Client::new(),
        };
        assert_eq!(client.auth_header().to_str().unwrap(), "Bearer my-secret");
    }
}
