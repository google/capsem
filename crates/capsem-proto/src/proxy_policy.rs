//! Path-free active-policy updates for a confined proxy worker.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROXY_SCHEMA_HASH: u64 = include!(concat!(env!("OUT_DIR"), "/proxy_schema_hash.txt"));
pub const MAX_ACTIVE_POLICY_BYTES: usize = 1024 * 1024;
pub const MAX_POLICY_ERROR_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyPolicyRequest {
    Apply {
        schema_hash: u64,
        request_id: u64,
        #[serde(with = "serde_bytes")]
        active_policy: Vec<u8>,
    },
}

impl ProxyPolicyRequest {
    pub fn validate(&self) -> Result<(), ProxyPolicyError> {
        match self {
            Self::Apply {
                schema_hash,
                request_id,
                active_policy,
            } if *schema_hash == PROXY_SCHEMA_HASH
                && *request_id != 0
                && !active_policy.is_empty()
                && active_policy.len() <= MAX_ACTIVE_POLICY_BYTES =>
            {
                Ok(())
            }
            _ => Err(ProxyPolicyError::InvalidRequest),
        }
    }

    pub fn apply(request_id: u64, active_policy: Vec<u8>) -> Self {
        Self::Apply {
            schema_hash: PROXY_SCHEMA_HASH,
            request_id,
            active_policy,
        }
    }

    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Apply { request_id, .. } => *request_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyPolicyResponse {
    Applied {
        request_id: u64,
        active_policy_digest: String,
    },
    Rejected {
        request_id: u64,
        error: String,
    },
}

impl ProxyPolicyResponse {
    pub fn rejected(request_id: u64, error: impl Into<String>) -> Self {
        let mut error = error.into();
        if error.len() > MAX_POLICY_ERROR_BYTES {
            let mut end = MAX_POLICY_ERROR_BYTES;
            while !error.is_char_boundary(end) {
                end -= 1;
            }
            error.truncate(end);
        }
        Self::Rejected { request_id, error }
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ProxyPolicyError {
    #[error("proxy policy request has an incompatible schema, zero id, or invalid active-policy body")]
    InvalidRequest,
}

#[cfg(test)]
mod tests;
