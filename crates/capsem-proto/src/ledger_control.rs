//! Fixed coordinator control records for one session ledger worker.
//!
//! The trusted coordinator launches a worker with one generation, then sends
//! connected client descriptors beside `Attach` records. The record contains
//! no session id or path: the process already owns exactly one ledger.

use thiserror::Error;

use crate::ledger::{
    LedgerChannelGrant, LedgerClientRole, LedgerGeneration, LedgerProtocolError, LEDGER_PROTOCOL_VERSION,
};

pub const LEDGER_CONTROL_FRAME_SIZE: usize = 32;
pub const LEDGER_CONTROL_MAX_FDS: usize = 1;

const MAGIC: [u8; 2] = *b"LG";
const VERSION_RANGE: std::ops::Range<usize> = 2..4;
const KIND_OFFSET: usize = 4;
const ROLE_OFFSET: usize = 5;
const DETAIL_RANGE: std::ops::Range<usize> = 6..8;
const GENERATION_RANGE: std::ops::Range<usize> = 8..24;
const CLIENT_ID_RANGE: std::ops::Range<usize> = 24..32;

const ATTACH: u8 = 1;
const SHUTDOWN: u8 = 2;
const READY: u8 = 101;
const ADOPTED: u8 = 102;
const REJECTED: u8 = 103;
const STOPPED: u8 = 104;
const CLOSED: u8 = 105;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerControlRequest {
    Attach(LedgerChannelGrant),
    Shutdown { generation: LedgerGeneration },
}

impl LedgerControlRequest {
    pub const fn generation(self) -> LedgerGeneration {
        match self {
            Self::Attach(grant) => grant.generation(),
            Self::Shutdown { generation } => generation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerControlEvent {
    Ready {
        generation: LedgerGeneration,
    },
    Adopted {
        generation: LedgerGeneration,
        client_id: u64,
    },
    Rejected {
        generation: LedgerGeneration,
        client_id: u64,
        reason: LedgerControlRejection,
    },
    Stopped {
        generation: LedgerGeneration,
    },
    Closed {
        generation: LedgerGeneration,
        client_id: u64,
        reason: LedgerClientCloseReason,
    },
}

impl LedgerControlEvent {
    pub const fn generation(self) -> LedgerGeneration {
        match self {
            Self::Ready { generation }
            | Self::Adopted { generation, .. }
            | Self::Rejected { generation, .. }
            | Self::Stopped { generation }
            | Self::Closed { generation, .. } => generation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum LedgerControlRejection {
    Capacity = 1,
    ShuttingDown = 2,
    DuplicateClient = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum LedgerClientCloseReason {
    Disconnected = 1,
    ShutdownRequested = 2,
    ProtocolError = 3,
}

pub fn encode_ledger_control_request(request: LedgerControlRequest) -> [u8; LEDGER_CONTROL_FRAME_SIZE] {
    match request {
        LedgerControlRequest::Attach(grant) => encode(
            ATTACH,
            role_code(grant.role()),
            0,
            grant.generation(),
            grant.client_id(),
        ),
        LedgerControlRequest::Shutdown { generation } => encode(SHUTDOWN, 0, 0, generation, 0),
    }
}

pub fn decode_ledger_control_request(
    frame: &[u8; LEDGER_CONTROL_FRAME_SIZE],
) -> Result<LedgerControlRequest, LedgerControlError> {
    validate_header(frame)?;
    let generation = decode_generation(frame);
    let client_id = decode_u64(frame, CLIENT_ID_RANGE);
    let detail = decode_u16(frame, DETAIL_RANGE);
    match frame[KIND_OFFSET] {
        ATTACH if detail == 0 => {
            let role = decode_role(frame[ROLE_OFFSET])?;
            Ok(LedgerControlRequest::Attach(
                LedgerChannelGrant::new(generation, client_id, role).map_err(LedgerControlError::InvalidGrant)?,
            ))
        }
        SHUTDOWN if frame[ROLE_OFFSET] == 0 && detail == 0 && client_id == 0 => {
            validate_generation(generation)?;
            Ok(LedgerControlRequest::Shutdown { generation })
        }
        kind => Err(LedgerControlError::InvalidRequest(kind)),
    }
}

pub fn encode_ledger_control_event(event: LedgerControlEvent) -> [u8; LEDGER_CONTROL_FRAME_SIZE] {
    match event {
        LedgerControlEvent::Ready { generation } => encode(READY, 0, 0, generation, 0),
        LedgerControlEvent::Adopted { generation, client_id } => encode(ADOPTED, 0, 0, generation, client_id),
        LedgerControlEvent::Rejected {
            generation,
            client_id,
            reason,
        } => encode(REJECTED, 0, reason as u16, generation, client_id),
        LedgerControlEvent::Stopped { generation } => encode(STOPPED, 0, 0, generation, 0),
        LedgerControlEvent::Closed {
            generation,
            client_id,
            reason,
        } => encode(CLOSED, 0, reason as u16, generation, client_id),
    }
}

pub fn decode_ledger_control_event(
    frame: &[u8; LEDGER_CONTROL_FRAME_SIZE],
) -> Result<LedgerControlEvent, LedgerControlError> {
    validate_header(frame)?;
    if frame[ROLE_OFFSET] != 0 {
        return Err(LedgerControlError::ReservedBytes);
    }
    let generation = decode_generation(frame);
    validate_generation(generation)?;
    let client_id = decode_u64(frame, CLIENT_ID_RANGE);
    let detail = decode_u16(frame, DETAIL_RANGE);
    match frame[KIND_OFFSET] {
        READY if detail == 0 && client_id == 0 => Ok(LedgerControlEvent::Ready { generation }),
        ADOPTED if detail == 0 && client_id != 0 => Ok(LedgerControlEvent::Adopted { generation, client_id }),
        REJECTED if client_id != 0 => Ok(LedgerControlEvent::Rejected {
            generation,
            client_id,
            reason: match detail {
                1 => LedgerControlRejection::Capacity,
                2 => LedgerControlRejection::ShuttingDown,
                3 => LedgerControlRejection::DuplicateClient,
                _ => return Err(LedgerControlError::InvalidRejection(detail)),
            },
        }),
        STOPPED if detail == 0 && client_id == 0 => Ok(LedgerControlEvent::Stopped { generation }),
        CLOSED if client_id != 0 => Ok(LedgerControlEvent::Closed {
            generation,
            client_id,
            reason: match detail {
                1 => LedgerClientCloseReason::Disconnected,
                2 => LedgerClientCloseReason::ShutdownRequested,
                3 => LedgerClientCloseReason::ProtocolError,
                _ => return Err(LedgerControlError::InvalidCloseReason(detail)),
            },
        }),
        kind => Err(LedgerControlError::InvalidEvent(kind)),
    }
}

fn encode(
    kind: u8,
    role: u8,
    detail: u16,
    generation: LedgerGeneration,
    client_id: u64,
) -> [u8; LEDGER_CONTROL_FRAME_SIZE] {
    let mut frame = [0; LEDGER_CONTROL_FRAME_SIZE];
    frame[..2].copy_from_slice(&MAGIC);
    frame[VERSION_RANGE].copy_from_slice(&LEDGER_PROTOCOL_VERSION.to_be_bytes());
    frame[KIND_OFFSET] = kind;
    frame[ROLE_OFFSET] = role;
    frame[DETAIL_RANGE].copy_from_slice(&detail.to_be_bytes());
    frame[GENERATION_RANGE].copy_from_slice(&generation.as_bytes());
    frame[CLIENT_ID_RANGE].copy_from_slice(&client_id.to_be_bytes());
    frame
}

fn validate_header(frame: &[u8; LEDGER_CONTROL_FRAME_SIZE]) -> Result<(), LedgerControlError> {
    if frame[..2] != MAGIC {
        return Err(LedgerControlError::Magic);
    }
    let version = decode_u16(frame, VERSION_RANGE);
    if version != LEDGER_PROTOCOL_VERSION {
        return Err(LedgerControlError::Version {
            expected: LEDGER_PROTOCOL_VERSION,
            actual: version,
        });
    }
    Ok(())
}

fn validate_generation(generation: LedgerGeneration) -> Result<(), LedgerControlError> {
    LedgerChannelGrant::new(generation, 1, LedgerClientRole::Reader)
        .map(|_| ())
        .map_err(LedgerControlError::InvalidGrant)
}

const fn role_code(role: LedgerClientRole) -> u8 {
    match role {
        LedgerClientRole::VmOwner => 1,
        LedgerClientRole::Proxy => 2,
        LedgerClientRole::Coordinator => 3,
        LedgerClientRole::Reader => 4,
        LedgerClientRole::Maintainer => 5,
        LedgerClientRole::Supervisor => 6,
    }
}

fn decode_role(role: u8) -> Result<LedgerClientRole, LedgerControlError> {
    match role {
        1 => Ok(LedgerClientRole::VmOwner),
        2 => Ok(LedgerClientRole::Proxy),
        3 => Ok(LedgerClientRole::Coordinator),
        4 => Ok(LedgerClientRole::Reader),
        5 => Ok(LedgerClientRole::Maintainer),
        6 => Ok(LedgerClientRole::Supervisor),
        _ => Err(LedgerControlError::InvalidRole(role)),
    }
}

fn decode_u16(frame: &[u8; LEDGER_CONTROL_FRAME_SIZE], range: std::ops::Range<usize>) -> u16 {
    u16::from_be_bytes(frame[range].try_into().expect("fixed ledger control range"))
}

fn decode_u64(frame: &[u8; LEDGER_CONTROL_FRAME_SIZE], range: std::ops::Range<usize>) -> u64 {
    u64::from_be_bytes(frame[range].try_into().expect("fixed ledger control range"))
}

fn decode_generation(frame: &[u8; LEDGER_CONTROL_FRAME_SIZE]) -> LedgerGeneration {
    LedgerGeneration::new(
        frame[GENERATION_RANGE]
            .try_into()
            .expect("fixed ledger generation range"),
    )
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum LedgerControlError {
    #[error("ledger control magic mismatch")]
    Magic,
    #[error("ledger control version mismatch: expected {expected}, got {actual}")]
    Version { expected: u16, actual: u16 },
    #[error("ledger control request kind {0} is invalid")]
    InvalidRequest(u8),
    #[error("ledger control event kind {0} is invalid")]
    InvalidEvent(u8),
    #[error("ledger control role {0} is invalid")]
    InvalidRole(u8),
    #[error("ledger control rejection {0} is invalid")]
    InvalidRejection(u16),
    #[error("ledger client close reason {0} is invalid")]
    InvalidCloseReason(u16),
    #[error("ledger control reserved bytes are nonzero")]
    ReservedBytes,
    #[error("ledger control grant is invalid: {0}")]
    InvalidGrant(LedgerProtocolError),
}

#[cfg(test)]
mod tests;
