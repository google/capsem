//! Generation-bound authority and envelopes for one session ledger channel.
//!
//! A trusted coordinator creates a connected channel and gives both endpoints
//! the same grant. The grant deliberately has no session id or path: the
//! worker already owns one pre-opened session directory, and possession of a
//! generation-bound descriptor selects it. After the hello exchange, ordinary
//! requests carry only a request id and a transport-independent operation.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Version of the ledger handshake, request envelopes and logger-owned
/// operation/reply enums. Bump it when any of those wire shapes changes.
/// Version 3 adds maintainer-only coherent snapshot authority.
pub const LEDGER_PROTOCOL_VERSION: u16 = 3;
/// Hash of ledger wire declarations, separate from the guest protocol hash.
pub const LEDGER_SCHEMA_HASH: u64 = include!(concat!(env!("OUT_DIR"), "/ledger_schema_hash.txt"));
/// Largest diagnostic admitted into a response.
pub const MAX_LEDGER_ERROR_BYTES: usize = 4 * 1024;

/// Fresh identity of one supervised ledger-owner lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerGeneration([u8; 16]);

impl LedgerGeneration {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }

    const fn is_zero(self) -> bool {
        let mut index = 0;
        while index < self.0.len() {
            if self.0[index] != 0 {
                return false;
            }
            index += 1;
        }
        true
    }
}

/// Trusted identity and authority stamped on one connected client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerClientRole {
    VmOwner,
    Proxy,
    Coordinator,
    Reader,
    Maintainer,
    Supervisor,
}

/// Classes used to authorize an operation before storage dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerCapability {
    Admit,
    Flush,
    Read,
    Retain,
    Export,
    Snapshot,
    Shutdown,
}

impl LedgerClientRole {
    pub const fn permits(self, capability: LedgerCapability) -> bool {
        match self {
            Self::VmOwner | Self::Proxy | Self::Coordinator => {
                matches!(capability, LedgerCapability::Admit | LedgerCapability::Flush)
            }
            Self::Reader => matches!(capability, LedgerCapability::Read | LedgerCapability::Export),
            Self::Maintainer => matches!(
                capability,
                LedgerCapability::Read
                    | LedgerCapability::Retain
                    | LedgerCapability::Export
                    | LedgerCapability::Snapshot
            ),
            Self::Supervisor => matches!(capability, LedgerCapability::Shutdown),
        }
    }

    /// Stable producer identity the worker stamps on accepted operations.
    pub const fn producer_name(self) -> Option<&'static str> {
        match self {
            Self::VmOwner => Some("vm_owner"),
            Self::Proxy => Some("proxy"),
            Self::Coordinator => Some("coordinator"),
            Self::Reader | Self::Maintainer | Self::Supervisor => None,
        }
    }
}

/// Coordinator-minted authority for one already connected channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LedgerChannelGrant {
    generation: LedgerGeneration,
    client_id: u64,
    role: LedgerClientRole,
}

impl LedgerChannelGrant {
    pub fn new(
        generation: LedgerGeneration,
        client_id: u64,
        role: LedgerClientRole,
    ) -> Result<Self, LedgerProtocolError> {
        if generation.is_zero() {
            return Err(LedgerProtocolError::ZeroGeneration);
        }
        if client_id == 0 {
            return Err(LedgerProtocolError::ZeroClientId);
        }
        Ok(Self {
            generation,
            client_id,
            role,
        })
    }

    pub const fn generation(self) -> LedgerGeneration {
        self.generation
    }

    pub const fn client_id(self) -> u64 {
        self.client_id
    }

    pub const fn role(self) -> LedgerClientRole {
        self.role
    }

    pub fn validate_hello(self, hello: &LedgerHello) -> Result<(), LedgerHandshakeError> {
        validate_compatibility(hello.protocol_version, hello.schema_hash)?;
        if hello.generation != self.generation {
            return Err(LedgerHandshakeError::Generation);
        }
        if hello.client_id != self.client_id {
            return Err(LedgerHandshakeError::ClientId);
        }
        Ok(())
    }

    pub fn validate_welcome(self, welcome: &LedgerWelcome) -> Result<(), LedgerHandshakeError> {
        validate_compatibility(welcome.protocol_version, welcome.schema_hash)?;
        if welcome.generation != self.generation {
            return Err(LedgerHandshakeError::Generation);
        }
        if welcome.client_id != self.client_id {
            return Err(LedgerHandshakeError::ClientId);
        }
        if welcome.role != self.role {
            return Err(LedgerHandshakeError::Role);
        }
        Ok(())
    }
}

/// First client-to-worker value on a freshly granted channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerHello {
    protocol_version: u16,
    schema_hash: u64,
    generation: LedgerGeneration,
    client_id: u64,
}

impl LedgerHello {
    pub const fn for_grant(grant: &LedgerChannelGrant) -> Self {
        Self {
            protocol_version: LEDGER_PROTOCOL_VERSION,
            schema_hash: LEDGER_SCHEMA_HASH,
            generation: grant.generation,
            client_id: grant.client_id,
        }
    }
}

/// Worker-to-client confirmation of the exact generation and granted role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerWelcome {
    protocol_version: u16,
    schema_hash: u64,
    generation: LedgerGeneration,
    client_id: u64,
    role: LedgerClientRole,
}

impl LedgerWelcome {
    pub const fn for_grant(grant: &LedgerChannelGrant) -> Self {
        Self {
            protocol_version: LEDGER_PROTOCOL_VERSION,
            schema_hash: LEDGER_SCHEMA_HASH,
            generation: grant.generation,
            client_id: grant.client_id,
            role: grant.role,
        }
    }
}

fn validate_compatibility(protocol_version: u16, schema_hash: u64) -> Result<(), LedgerHandshakeError> {
    if protocol_version != LEDGER_PROTOCOL_VERSION {
        return Err(LedgerHandshakeError::ProtocolVersion {
            expected: LEDGER_PROTOCOL_VERSION,
            actual: protocol_version,
        });
    }
    if schema_hash != LEDGER_SCHEMA_HASH {
        return Err(LedgerHandshakeError::SchemaHash {
            expected: LEDGER_SCHEMA_HASH,
            actual: schema_hash,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum LedgerHandshakeError {
    #[error("ledger protocol version mismatch: expected {expected}, got {actual}")]
    ProtocolVersion { expected: u16, actual: u16 },
    #[error("ledger schema hash mismatch: expected {expected:#x}, got {actual:#x}")]
    SchemaHash { expected: u64, actual: u64 },
    #[error("ledger generation does not match the granted channel")]
    Generation,
    #[error("ledger client id does not match the granted channel")]
    ClientId,
    #[error("ledger role does not match the granted channel")]
    Role,
}

/// An operation declares the authority class checked before it is dispatched.
pub trait LedgerOperation {
    fn capability(&self) -> LedgerCapability;

    fn validate(&self) -> Result<(), LedgerProtocolError> {
        Ok(())
    }
}

/// One post-handshake operation. Session and generation are channel state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRequest<O> {
    request_id: u64,
    operation: O,
}

impl<O> LedgerRequest<O> {
    pub fn new(request_id: u64, operation: O) -> Result<Self, LedgerProtocolError> {
        if request_id == 0 {
            return Err(LedgerProtocolError::ZeroRequestId);
        }
        Ok(Self { request_id, operation })
    }

    pub const fn request_id(&self) -> u64 {
        self.request_id
    }

    pub const fn operation(&self) -> &O {
        &self.operation
    }

    pub fn into_operation(self) -> O {
        self.operation
    }
}

impl<O: LedgerOperation> LedgerRequest<O> {
    pub fn authorize(&self, grant: &LedgerChannelGrant) -> Result<(), LedgerProtocolError> {
        if self.request_id == 0 {
            return Err(LedgerProtocolError::ZeroRequestId);
        }
        self.operation.validate()?;
        let capability = self.operation.capability();
        if !grant.role.permits(capability) {
            return Err(LedgerProtocolError::Unauthorized {
                role: grant.role,
                capability,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerFailureCode {
    InvalidRequest,
    UnauthorizedOperation,
    Busy,
    Unavailable,
    Storage,
    CorruptLedger,
    LimitExceeded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerFailure {
    code: LedgerFailureCode,
    message: String,
}

impl LedgerFailure {
    pub fn new(code: LedgerFailureCode, message: impl Into<String>) -> Result<Self, LedgerProtocolError> {
        let message = message.into();
        if message.len() > MAX_LEDGER_ERROR_BYTES {
            return Err(LedgerProtocolError::ErrorMessageTooLong);
        }
        Ok(Self { code, message })
    }

    pub const fn code(&self) -> LedgerFailureCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerOutcome<R> {
    Success { reply: R },
    Failure { failure: LedgerFailure },
}

/// One correlated reply. Streaming operations use several replies with the
/// same request id and bounded operation-specific chunks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerResponse<R> {
    request_id: u64,
    outcome: LedgerOutcome<R>,
}

impl<R> LedgerResponse<R> {
    pub fn success(request_id: u64, reply: R) -> Result<Self, LedgerProtocolError> {
        Self::new(request_id, LedgerOutcome::Success { reply })
    }

    pub fn failure(request_id: u64, failure: LedgerFailure) -> Result<Self, LedgerProtocolError> {
        Self::new(request_id, LedgerOutcome::Failure { failure })
    }

    fn new(request_id: u64, outcome: LedgerOutcome<R>) -> Result<Self, LedgerProtocolError> {
        if request_id == 0 {
            return Err(LedgerProtocolError::ZeroRequestId);
        }
        Ok(Self { request_id, outcome })
    }

    pub const fn request_id(&self) -> u64 {
        self.request_id
    }

    pub const fn outcome(&self) -> &LedgerOutcome<R> {
        &self.outcome
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum LedgerProtocolError {
    #[error("ledger generation must not be zero")]
    ZeroGeneration,
    #[error("ledger client id must not be zero")]
    ZeroClientId,
    #[error("ledger request id must not be zero")]
    ZeroRequestId,
    #[error("ledger error message exceeds its bound")]
    ErrorMessageTooLong,
    #[error("ledger operation fields are invalid")]
    InvalidOperation,
    #[error("encoded ledger operation exceeds its bound")]
    OperationTooLarge,
    #[error("ledger role {role:?} cannot perform {capability:?}")]
    Unauthorized {
        role: LedgerClientRole,
        capability: LedgerCapability,
    },
}

#[cfg(test)]
mod tests;
