use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use capsem_proto::mcp::McpAuthConfig;

// ---------------------------------------------------------------------------
// MCP server config (stored under [mcp])
// ---------------------------------------------------------------------------

/// MCP configuration from the `[mcp]` section of settings.toml or corp.toml.
///
/// This is server discovery/configuration only. MCP allow/ask/block decisions
/// are security rules over canonical MCP security events.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    /// Health check interval in seconds (default: 300).
    #[serde(default)]
    pub health_check_interval_secs: Option<u64>,
    /// Manually configured MCP servers.
    #[serde(default)]
    pub servers: Vec<McpManualServer>,
    /// Per-server enabled overrides (name -> enabled).
    #[serde(default)]
    pub server_enabled: HashMap<String, bool>,
}

/// A manually configured MCP server definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct McpManualServer {
    pub name: String,
    /// HTTP endpoint URL for the MCP server.
    pub url: String,
    /// Custom HTTP headers to send with every request.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Brokered auth material for the remote MCP server.
    #[serde(default)]
    pub auth: Option<McpAuthConfig>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl McpConfig {
    pub fn validate(&self, context: &str) -> Result<(), String> {
        for server in &self.servers {
            server.validate(context)?;
        }
        Ok(())
    }

    /// The MCP configuration a session runs with: the user's `[mcp]` with the
    /// corp `[mcp]` laid over it. A corp server replaces the user's server of
    /// the same name, a corp `server_enabled` entry wins, and the corp health
    /// interval wins when set. `None` when neither file has an `[mcp]`.
    pub fn merged(user: Option<&Self>, corp: Option<&Self>) -> Option<Self> {
        let (mut merged, corp) = match (user, corp) {
            (None, None) => return None,
            (Some(user), None) => return Some(user.clone()),
            (None, Some(corp)) => return Some(corp.clone()),
            (Some(user), Some(corp)) => (user.clone(), corp),
        };
        merged
            .servers
            .retain(|server| !corp.servers.iter().any(|locked| locked.name == server.name));
        merged.servers.extend(corp.servers.iter().cloned());
        for (name, enabled) in &corp.server_enabled {
            merged.server_enabled.insert(name.clone(), *enabled);
        }
        if corp.health_check_interval_secs.is_some() {
            merged.health_check_interval_secs = corp.health_check_interval_secs;
        }
        Some(merged)
    }
}

impl McpManualServer {
    fn validate(&self, context: &str) -> Result<(), String> {
        for key in self.headers.keys() {
            if is_secret_header(key) {
                return Err(format!(
                    "{context}.mcp.servers.{}.headers.{key} is secret-bearing; use auth.credential_ref through the credential broker",
                    self.name
                ));
            }
        }
        if let Some(auth) = &self.auth {
            if !crate::is_credential_reference(&auth.credential_ref) {
                return Err(format!(
                    "{context}.mcp.servers.{}.auth.credential_ref must be a credential:blake3 reference",
                    self.name
                ));
            }
        }
        Ok(())
    }
}

pub fn is_secret_header(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key == "authorization"
        || key == "proxy-authorization"
        || key == "x-api-key"
        || key == "api-key"
        || key == "x-auth-token"
        || key.ends_with("-token")
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests;
