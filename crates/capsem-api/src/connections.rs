//! Reviewed host connection and session-grant contracts, without token exports.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

const MAX_SAFE_REVISION: u64 = 9_007_199_254_740_991;

fn deserialize_revision<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let value = u64::deserialize(deserializer)?;
    if value > MAX_SAFE_REVISION {
        return Err(serde::de::Error::custom(
            "credential revision exceeds safe integer range",
        ));
    }
    Ok(value)
}

fn serialize_revision<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    if *value > MAX_SAFE_REVISION {
        return Err(serde::ser::Error::custom(
            "credential revision exceeds safe integer range",
        ));
    }
    serializer.serialize_u64(*value)
}

fn deserialize_uuid<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if !crate::identity::canonical_uuid(&value) {
        return Err(serde::de::Error::custom(
            "credential ownership id must be a canonical UUID",
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Connected,
    ReauthRequired,
    Disconnecting,
    Disconnected,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Pending,
    Connected,
    Denied,
    Expired,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GrantState {
    Active,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRevocationState {
    NotStarted,
    Pending,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialErrorCode {
    ConfigurationRequired,
    UnsupportedCapability,
    UnknownScopeProfile,
    PolicyDenied,
    ScopeDenied,
    ConnectionUnavailable,
    ReauthRequired,
    StorageUnavailable,
    StorageCommitFailed,
    TransactionExpired,
    TransactionCancelled,
    InvalidCallback,
    ProviderRejected,
    ProviderTimeout,
    OwnerChanged,
    SessionUnavailable,
    GrantUnavailable,
    RequestConflict,
    RevocationPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentKind {
    OwnedOauthClient,
    NativeConsumerSignIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionProvider {
    Google,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySummary {
    pub id: String,
    pub label: String,
    pub consumer: String,
    pub required_scopes: Vec<String>,
    pub supported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScopeProfileSummary {
    pub id: String,
    pub enrollment_kind: EnrollmentKind,
    pub label: String,
    pub application_label: String,
    pub requested_scopes: Vec<String>,
    pub capabilities: Vec<CapabilitySummary>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionSummary {
    pub id: String,
    pub provider: ConnectionProvider,
    pub scope_profile_id: String,
    #[schema(required = true)]
    pub account_label: Option<String>,
    pub state: ConnectionState,
    #[serde(deserialize_with = "deserialize_revision", serialize_with = "serialize_revision")]
    #[schema(maximum = 9007199254740991_u64)]
    pub revision: u64,
    pub granted_scopes: Vec<String>,
    pub available_capabilities: Vec<String>,
    #[schema(format = "date-time")]
    pub created_at: String,
    #[schema(format = "date-time")]
    pub updated_at: String,
    #[schema(required = true, format = "date-time")]
    pub last_refresh_at: Option<String>,
    pub provider_revocation: ProviderRevocationState,
    #[schema(required = true)]
    pub error_code: Option<CredentialErrorCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionListResponse {
    pub connections: Vec<ConnectionSummary>,
    pub scope_profiles: Vec<ScopeProfileSummary>,
    #[schema(required = true)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthorizeGoogleRequest {
    #[serde(deserialize_with = "deserialize_uuid")]
    #[schema(format = "uuid")]
    pub request_id: String,
    pub scope_profile_id: String,
    #[schema(required = true)]
    pub reconnect_connection_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationTransaction {
    pub id: String,
    pub state: TransactionState,
    #[schema(format = "date-time")]
    pub expires_at: String,
    #[schema(required = true)]
    pub connection_id: Option<String>,
    #[schema(required = true)]
    pub error_code: Option<CredentialErrorCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthorizeGoogleResponse {
    pub transaction: AuthorizationTransaction,
    #[schema(required = true)]
    pub authorization_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocalCredentialAuthority {
    Revoking,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DisconnectResponse {
    pub connection: ConnectionSummary,
    pub revoked_grant_count: u32,
    pub local_authority: LocalCredentialAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionGrantRequest {
    #[serde(deserialize_with = "deserialize_uuid")]
    #[schema(format = "uuid")]
    pub request_id: String,
    #[serde(deserialize_with = "deserialize_uuid")]
    #[schema(format = "uuid")]
    pub expected_owner_generation: String,
    pub connection_id: String,
    pub capability_ids: Vec<String>,
    #[schema(required = true, format = "date-time")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionGrantSummary {
    pub id: String,
    pub session_id: String,
    pub owner_generation: String,
    pub connection_id: String,
    pub capability_ids: Vec<String>,
    pub state: GrantState,
    #[serde(deserialize_with = "deserialize_revision", serialize_with = "serialize_revision")]
    #[schema(maximum = 9007199254740991_u64)]
    pub revision: u64,
    #[schema(format = "date-time")]
    pub created_at: String,
    #[schema(format = "date-time")]
    pub expires_at: String,
    #[schema(required = true, format = "date-time")]
    pub revoked_at: Option<String>,
    pub runtime_bound: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionGrantListResponse {
    pub session_id: String,
    pub owner_generation: String,
    pub grants: Vec<SessionGrantSummary>,
    pub denied_capability_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevokedCredentialAuthority {
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeSessionGrantResponse {
    pub grant: SessionGrantSummary,
    pub local_authority: RevokedCredentialAuthority,
}

#[cfg(test)]
mod tests;
