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
mod tests {
    use super::*;

    #[test]
    fn requests_and_correlated_answers_are_bounded() {
        let address = ProxyPrivateNameRequest::AddressOf {
            request_id: 1,
            name: "box.dev.capsem.internal".to_string(),
        };
        let name = ProxyPrivateNameRequest::NameOf {
            request_id: 2,
            address: Ipv4Addr::new(10, 0, 0, 2),
        };
        address.validate().unwrap();
        name.validate().unwrap();
        assert_eq!(address.request_id(), 1);
        assert_eq!(name.request_id(), 2);
        assert_eq!(
            ProxyPrivateNameResponse::Address {
                request_id: 1,
                address: Ipv4Addr::new(10, 0, 0, 2),
            }
            .request_id(),
            1
        );
    }

    #[test]
    fn malformed_authority_and_oversized_values_fail_closed() {
        assert!(ProxyPrivateNameRequest::AddressOf {
            request_id: 0,
            name: String::new(),
        }
        .validate()
        .is_err());
        assert!(ProxyPrivateNameRequest::AddressOf {
            request_id: 1,
            name: "x".repeat(MAX_PRIVATE_NAME_BYTES + 1),
        }
        .validate()
        .is_err());
        assert!(ProxyPrivateNameResponse::rejected(1, "x".repeat(MAX_PRIVATE_NAME_ERROR_BYTES + 1)).is_err());
    }
}
