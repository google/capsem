//! Bootstrap metadata for one proxy worker's MCP capability.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROXY_MCP_VERSION: u16 = 1;
pub const PROXY_MCP_MAX_BUILTIN_SERVERS: usize = 64;
pub const PROXY_MCP_MAX_SERVER_NAME_BYTES: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyMcpHello {
    pub version: u16,
    pub builtin_servers: Vec<String>,
}

impl ProxyMcpHello {
    pub fn new(builtin_servers: Vec<String>) -> Result<Self, ProxyMcpError> {
        let hello = Self {
            version: PROXY_MCP_VERSION,
            builtin_servers,
        };
        hello.validate()?;
        Ok(hello)
    }

    pub fn validate(&self) -> Result<(), ProxyMcpError> {
        if self.version != PROXY_MCP_VERSION {
            return Err(ProxyMcpError::Version {
                expected: PROXY_MCP_VERSION,
                actual: self.version,
            });
        }
        if self.builtin_servers.len() > PROXY_MCP_MAX_BUILTIN_SERVERS {
            return Err(ProxyMcpError::TooManyBuiltinServers(self.builtin_servers.len()));
        }
        let mut seen = HashSet::with_capacity(self.builtin_servers.len());
        for name in &self.builtin_servers {
            if name.is_empty() || name.len() > PROXY_MCP_MAX_SERVER_NAME_BYTES {
                return Err(ProxyMcpError::InvalidServerName);
            }
            if !seen.insert(name) {
                return Err(ProxyMcpError::DuplicateServerName);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ProxyMcpError {
    #[error("unsupported proxy MCP version {actual}, expected {expected}")]
    Version { expected: u16, actual: u16 },
    #[error("proxy MCP capability names {0} builtin servers, exceeding the limit")]
    TooManyBuiltinServers(usize),
    #[error("proxy MCP capability contains an invalid builtin server name")]
    InvalidServerName,
    #[error("proxy MCP capability contains a duplicate builtin server name")]
    DuplicateServerName,
}

#[cfg(test)]
mod tests;
