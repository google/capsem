use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_PRIVATE_NAME_BYTES: usize = 253;
pub const MAX_PRIVATE_NAME_ERROR_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyPrivateNameRequest {
    AddressOf { request_id: u64, name: String },
    NameOf { request_id: u64, address: Ipv4Addr },
}

impl ProxyPrivateNameRequest {
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::AddressOf { request_id, .. } | Self::NameOf { request_id, .. } => *request_id,
        }
    }

    pub fn validate(&self) -> Result<(), ProxyPrivateNameProtocolError> {
        if self.request_id() == 0 {
            return Err(ProxyPrivateNameProtocolError::ZeroRequestId);
        }
        if let Self::AddressOf { name, .. } = self {
            if name.is_empty() || name.len() > MAX_PRIVATE_NAME_BYTES {
                return Err(ProxyPrivateNameProtocolError::InvalidName);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyPrivateNameResponse {
    Address { request_id: u64, address: Ipv4Addr },
    Name { request_id: u64, name: String },
    NotFound { request_id: u64 },
    Rejected { request_id: u64, error: String },
}

impl ProxyPrivateNameResponse {
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Address { request_id, .. }
            | Self::Name { request_id, .. }
            | Self::NotFound { request_id }
            | Self::Rejected { request_id, .. } => *request_id,
        }
    }

    pub fn rejected(request_id: u64, error: impl Into<String>) -> Result<Self, ProxyPrivateNameProtocolError> {
        if request_id == 0 {
            return Err(ProxyPrivateNameProtocolError::ZeroRequestId);
        }
        let error = error.into();
        if error.len() > MAX_PRIVATE_NAME_ERROR_BYTES {
            return Err(ProxyPrivateNameProtocolError::ErrorTooLong);
        }
        Ok(Self::Rejected { request_id, error })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProxyPrivateNameProtocolError {
    #[error("proxy private-name request id must not be zero")]
    ZeroRequestId,
    #[error("proxy private name is empty or exceeds its bound")]
    InvalidName,
    #[error("proxy private-name diagnostic exceeds its bound")]
    ErrorTooLong,
}

#[cfg(test)]
mod tests;
