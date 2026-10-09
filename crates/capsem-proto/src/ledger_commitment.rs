//! Producer-authored commitments anchored by the trusted ledger coordinator.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration, LedgerHello, LedgerWelcome};

pub const MAX_COMMITMENT_EVENT_KIND_BYTES: usize = 64;
pub const MAX_COMMITMENTS_PER_CHECKPOINT: usize = 4096;
pub const ZERO_COMMITMENT_HASH: [u8; 32] = [0; 32];

/// Identity, order and content commitment for one producer record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerCommitment {
    generation: LedgerGeneration,
    client_id: u64,
    role: LedgerClientRole,
    producer_sequence: u64,
    global_sequence: u64,
    event_kind: String,
    event_hash: [u8; 32],
    previous_hash: [u8; 32],
    commitment_hash: [u8; 32],
}

impl LedgerCommitment {
    pub fn new(
        grant: LedgerChannelGrant,
        producer_sequence: u64,
        global_sequence: u64,
        event_kind: impl Into<String>,
        event_hash: [u8; 32],
        previous_hash: [u8; 32],
    ) -> Result<Self, LedgerCommitmentError> {
        let mut commitment = Self {
            generation: grant.generation(),
            client_id: grant.client_id(),
            role: grant.role(),
            producer_sequence,
            global_sequence,
            event_kind: event_kind.into(),
            event_hash,
            previous_hash,
            commitment_hash: ZERO_COMMITMENT_HASH,
        };
        commitment.validate_fields()?;
        commitment.commitment_hash = commitment.calculate_hash();
        Ok(commitment)
    }

    pub fn validate(&self) -> Result<(), LedgerCommitmentError> {
        self.validate_fields()?;
        if self.calculate_hash() != self.commitment_hash {
            return Err(LedgerCommitmentError::HashMismatch);
        }
        Ok(())
    }

    pub fn validate_grant(&self, grant: LedgerChannelGrant) -> Result<(), LedgerCommitmentError> {
        self.validate()?;
        if self.generation != grant.generation() || self.client_id != grant.client_id() || self.role != grant.role() {
            return Err(LedgerCommitmentError::AuthorityMismatch);
        }
        Ok(())
    }

    pub const fn generation(&self) -> LedgerGeneration {
        self.generation
    }

    pub const fn client_id(&self) -> u64 {
        self.client_id
    }

    pub const fn role(&self) -> LedgerClientRole {
        self.role
    }

    pub const fn producer_sequence(&self) -> u64 {
        self.producer_sequence
    }

    pub const fn global_sequence(&self) -> u64 {
        self.global_sequence
    }

    pub fn event_kind(&self) -> &str {
        &self.event_kind
    }

    pub const fn event_hash(&self) -> [u8; 32] {
        self.event_hash
    }

    pub const fn previous_hash(&self) -> [u8; 32] {
        self.previous_hash
    }

    pub const fn commitment_hash(&self) -> [u8; 32] {
        self.commitment_hash
    }

    fn validate_fields(&self) -> Result<(), LedgerCommitmentError> {
        if self.client_id == 0 || self.producer_sequence == 0 || self.global_sequence == 0 {
            return Err(LedgerCommitmentError::ZeroSequence);
        }
        if self.role.producer_name().is_none() {
            return Err(LedgerCommitmentError::NonProducer);
        }
        if self.event_kind.is_empty() || self.event_kind.len() > MAX_COMMITMENT_EVENT_KIND_BYTES {
            return Err(LedgerCommitmentError::InvalidEventKind);
        }
        Ok(())
    }

    fn calculate_hash(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new();
        hash.update(b"capsem-ledger-commitment-v1\0");
        hash.update(&self.generation.as_bytes());
        hash.update(&self.client_id.to_be_bytes());
        hash.update(&[role_tag(self.role)]);
        hash.update(&self.producer_sequence.to_be_bytes());
        hash.update(&self.global_sequence.to_be_bytes());
        hash.update(&(self.event_kind.len() as u64).to_be_bytes());
        hash.update(self.event_kind.as_bytes());
        hash.update(&self.event_hash);
        hash.update(&self.previous_hash);
        *hash.finalize().as_bytes()
    }
}

fn role_tag(role: LedgerClientRole) -> u8 {
    match role {
        LedgerClientRole::VmOwner => 1,
        LedgerClientRole::Proxy => 2,
        LedgerClientRole::Coordinator => 3,
        LedgerClientRole::Reader => 4,
        LedgerClientRole::Maintainer => 5,
        LedgerClientRole::Supervisor => 6,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentCommand {
    Reserve {
        producer_sequence: u64,
        event_kind: String,
        event_hash: [u8; 32],
        previous_hash: [u8; 32],
    },
    Cancel {
        global_sequence: u64,
    },
    Anchor {
        commitments: Vec<LedgerCommitment>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentReply {
    Reserved { global_sequence: u64 },
    Canceled,
    Anchored { checkpoint_sequence: u64 },
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentClientMessage {
    Hello {
        hello: LedgerHello,
    },
    Request {
        request_id: u64,
        command: CommitmentCommand,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentServerMessage {
    Welcome { welcome: LedgerWelcome },
    Response { request_id: u64, reply: CommitmentReply },
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum LedgerCommitmentError {
    #[error("ledger commitment sequence must not be zero")]
    ZeroSequence,
    #[error("ledger commitment role is not a producer")]
    NonProducer,
    #[error("ledger commitment event kind is invalid")]
    InvalidEventKind,
    #[error("ledger commitment hash does not match its fields")]
    HashMismatch,
    #[error("ledger commitment authority does not match its channel")]
    AuthorityMismatch,
}

#[cfg(test)]
mod tests;
