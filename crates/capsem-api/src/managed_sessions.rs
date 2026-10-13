//! User-approved managed ownership contracts; capabilities never appear in responses.

use capsem_crypto::secrets::Zeroizing;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

/// Lowercase hex encodes the client's 256-bit capability without URL transport.
#[derive(Clone)]
pub struct ManagedOwnershipToken(Zeroizing<String>);

impl ManagedOwnershipToken {
    pub fn new(value: String) -> Result<Self, &'static str> {
        let value = Zeroizing::new(value);
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("invalid managed ownership token");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ManagedOwnershipToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagedOwnershipToken([REDACTED])")
    }
}

impl Serialize for ManagedOwnershipToken {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ManagedOwnershipToken {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

fn deserialize_request_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || bytes[14] != b'4'
        || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        || !bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
            }
        })
    {
        return Err(serde::de::Error::custom(
            "managed request id must be a canonical UUIDv4",
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedSessionCreate {
    #[serde(deserialize_with = "deserialize_request_id")]
    #[schema(format = "uuid")]
    pub request_id: String,
    #[schema(value_type = String, write_only = true, pattern = "^[0-9a-f]{64}$")]
    pub ownership_token: ManagedOwnershipToken,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedSessionClaimRequest {
    #[schema(value_type = String, write_only = true, pattern = "^[0-9a-f]{64}$")]
    pub ownership_token: ManagedOwnershipToken,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedSessionCloseRequest {
    #[schema(value_type = String, write_only = true, pattern = "^[0-9a-f]{64}$")]
    pub ownership_token: ManagedOwnershipToken,
    pub outcome: ManagedSessionOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSessionOutcome {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSessionState {
    Pending,
    Active,
    Closing,
    Closed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSessionClosedState {
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedSessionOwnership {
    pub request_id: String,
    pub generation: String,
    pub expires_at: String,
    pub lease_seconds: u32,
    pub renew_after_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedSessionClaimResponse {
    pub ownership: ManagedSessionOwnership,
    pub state: ManagedSessionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<crate::ProvisionResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ManagedSessionFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedSessionCloseResponse {
    pub request_id: String,
    pub generation: String,
    pub state: ManagedSessionClosedState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub evidence_retained: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSessionErrorCode {
    InvalidRequest,
    Forbidden,
    NotFound,
    DuplicateRequest,
    Expired,
    CreationFailed,
    CleanupFailed,
    RecoveryPending,
    StoreUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedSessionFailure {
    pub code: ManagedSessionErrorCode,
    pub error: String,
}

#[cfg(test)]
mod tests;
