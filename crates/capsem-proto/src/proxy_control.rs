//! Fixed coordinator control records for one confined proxy worker.

use thiserror::Error;

use crate::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration, LedgerProtocolError};

pub const PROXY_CONTROL_FRAME_SIZE: usize = 56;
pub const PROXY_CONTROL_MAX_FDS: usize = 2;
pub const PROXY_CONTROL_VERSION: u16 = 6;

const MAGIC: [u8; 2] = *b"PX";
const VERSION_RANGE: std::ops::Range<usize> = 2..4;
const KIND_OFFSET: usize = 4;
const CAPABILITY_OFFSET: usize = 5;
const DETAIL_RANGE: std::ops::Range<usize> = 6..8;
const GENERATION_RANGE: std::ops::Range<usize> = 8..24;
const GRANT_ID_RANGE: std::ops::Range<usize> = 24..32;
const LEDGER_GENERATION_RANGE: std::ops::Range<usize> = 32..48;
const LEDGER_CLIENT_ID_RANGE: std::ops::Range<usize> = 48..56;

const ATTACH: u8 = 1;
const SHUTDOWN: u8 = 2;
const READY: u8 = 101;
const ADOPTED: u8 = 102;
const REJECTED: u8 = 103;
const STOPPED: u8 = 104;
const CLOSED: u8 = 105;

/// Fresh identity of one supervised proxy-worker lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProxyGeneration([u8; 16]);

impl ProxyGeneration {
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

/// Connected capabilities the coordinator may grant to a proxy worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum ProxyCapability {
    HttpTraffic = 1,
    DnsTraffic = 2,
    Upstream = 3,
    Credential = 4,
    Ledger = 5,
    PrivateNames = 6,
    Mcp = 7,
    Telemetry = 8,
    Policy = 9,
    TraceHints = 10,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProxyChannelGrant {
    generation: ProxyGeneration,
    grant_id: u64,
    capability: ProxyCapability,
    ledger_grant: Option<LedgerChannelGrant>,
}

impl ProxyChannelGrant {
    pub fn new(
        generation: ProxyGeneration,
        grant_id: u64,
        capability: ProxyCapability,
    ) -> Result<Self, ProxyControlError> {
        if generation.is_zero() {
            return Err(ProxyControlError::ZeroGeneration);
        }
        if grant_id == 0 {
            return Err(ProxyControlError::ZeroGrantId);
        }
        if capability == ProxyCapability::Ledger {
            return Err(ProxyControlError::MissingLedgerAuthority);
        }
        Ok(Self {
            generation,
            grant_id,
            capability,
            ledger_grant: None,
        })
    }

    pub fn with_ledger(
        generation: ProxyGeneration,
        grant_id: u64,
        ledger_grant: LedgerChannelGrant,
    ) -> Result<Self, ProxyControlError> {
        if ledger_grant.role() != LedgerClientRole::Proxy {
            return Err(ProxyControlError::InvalidLedgerRole(ledger_grant.role()));
        }
        if generation.is_zero() {
            return Err(ProxyControlError::ZeroGeneration);
        }
        if grant_id == 0 {
            return Err(ProxyControlError::ZeroGrantId);
        }
        Ok(Self {
            generation,
            grant_id,
            capability: ProxyCapability::Ledger,
            ledger_grant: Some(ledger_grant),
        })
    }

    pub const fn generation(self) -> ProxyGeneration {
        self.generation
    }

    pub const fn grant_id(self) -> u64 {
        self.grant_id
    }

    pub const fn capability(self) -> ProxyCapability {
        self.capability
    }

    pub const fn ledger_grant(self) -> Option<LedgerChannelGrant> {
        self.ledger_grant
    }

    /// Ledger writers receive the storage channel and its independent trusted
    /// checkpoint channel. Every other capability owns one descriptor.
    pub const fn expected_descriptor_count(self) -> usize {
        if matches!(self.capability, ProxyCapability::Ledger) {
            2
        } else {
            1
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyControlRequest {
    Attach(ProxyChannelGrant),
    Shutdown { generation: ProxyGeneration },
}

impl ProxyControlRequest {
    pub const fn generation(self) -> ProxyGeneration {
        match self {
            Self::Attach(grant) => grant.generation(),
            Self::Shutdown { generation } => generation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum ProxyControlRejection {
    Capacity = 1,
    DuplicateCapability = 2,
    DuplicateGrant = 3,
    ShuttingDown = 4,
    NotReady = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum ProxyChannelCloseReason {
    Disconnected = 1,
    ProtocolError = 2,
    Revoked = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyControlEvent {
    Ready {
        generation: ProxyGeneration,
    },
    Adopted {
        generation: ProxyGeneration,
        grant_id: u64,
    },
    Rejected {
        generation: ProxyGeneration,
        grant_id: u64,
        reason: ProxyControlRejection,
    },
    Closed {
        generation: ProxyGeneration,
        grant_id: u64,
        reason: ProxyChannelCloseReason,
    },
    Stopped {
        generation: ProxyGeneration,
    },
}

impl ProxyControlEvent {
    pub const fn generation(self) -> ProxyGeneration {
        match self {
            Self::Ready { generation }
            | Self::Adopted { generation, .. }
            | Self::Rejected { generation, .. }
            | Self::Closed { generation, .. }
            | Self::Stopped { generation } => generation,
        }
    }
}

pub fn encode_proxy_control_request(request: ProxyControlRequest) -> [u8; PROXY_CONTROL_FRAME_SIZE] {
    match request {
        ProxyControlRequest::Attach(grant) => {
            let mut frame = encode(
                ATTACH,
                grant.capability() as u8,
                grant.ledger_grant().map_or(0, |ledger| ledger_role_code(ledger.role())),
                grant.generation(),
                grant.grant_id(),
            );
            if let Some(ledger) = grant.ledger_grant() {
                frame[LEDGER_GENERATION_RANGE].copy_from_slice(&ledger.generation().as_bytes());
                frame[LEDGER_CLIENT_ID_RANGE].copy_from_slice(&ledger.client_id().to_be_bytes());
            }
            frame
        }
        ProxyControlRequest::Shutdown { generation } => encode(SHUTDOWN, 0, 0, generation, 0),
    }
}

pub fn decode_proxy_control_request(
    frame: &[u8; PROXY_CONTROL_FRAME_SIZE],
) -> Result<ProxyControlRequest, ProxyControlError> {
    validate_header(frame)?;
    let generation = decode_generation(frame);
    let grant_id = decode_u64(frame, GRANT_ID_RANGE);
    let detail = decode_u16(frame, DETAIL_RANGE);
    match frame[KIND_OFFSET] {
        ATTACH => {
            let capability = decode_capability(frame[CAPABILITY_OFFSET])?;
            let grant = if capability == ProxyCapability::Ledger {
                let ledger_generation = LedgerGeneration::new(
                    frame[LEDGER_GENERATION_RANGE]
                        .try_into()
                        .expect("fixed ledger generation range"),
                );
                let ledger_client_id = decode_u64(frame, LEDGER_CLIENT_ID_RANGE);
                let ledger = LedgerChannelGrant::new(ledger_generation, ledger_client_id, decode_ledger_role(detail)?)?;
                ProxyChannelGrant::with_ledger(generation, grant_id, ledger)?
            } else {
                if detail != 0
                    || frame[LEDGER_GENERATION_RANGE].iter().any(|byte| *byte != 0)
                    || decode_u64(frame, LEDGER_CLIENT_ID_RANGE) != 0
                {
                    return Err(ProxyControlError::ReservedBytes);
                }
                ProxyChannelGrant::new(generation, grant_id, capability)?
            };
            Ok(ProxyControlRequest::Attach(grant))
        }
        SHUTDOWN
            if frame[CAPABILITY_OFFSET] == 0
                && detail == 0
                && grant_id == 0
                && frame[LEDGER_GENERATION_RANGE].iter().all(|byte| *byte == 0)
                && decode_u64(frame, LEDGER_CLIENT_ID_RANGE) == 0 =>
        {
            validate_generation(generation)?;
            Ok(ProxyControlRequest::Shutdown { generation })
        }
        kind => Err(ProxyControlError::InvalidRequest(kind)),
    }
}

pub fn encode_proxy_control_event(event: ProxyControlEvent) -> [u8; PROXY_CONTROL_FRAME_SIZE] {
    match event {
        ProxyControlEvent::Ready { generation } => encode(READY, 0, 0, generation, 0),
        ProxyControlEvent::Adopted { generation, grant_id } => encode(ADOPTED, 0, 0, generation, grant_id),
        ProxyControlEvent::Rejected {
            generation,
            grant_id,
            reason,
        } => encode(REJECTED, 0, reason as u16, generation, grant_id),
        ProxyControlEvent::Closed {
            generation,
            grant_id,
            reason,
        } => encode(CLOSED, 0, reason as u16, generation, grant_id),
        ProxyControlEvent::Stopped { generation } => encode(STOPPED, 0, 0, generation, 0),
    }
}

pub fn decode_proxy_control_event(
    frame: &[u8; PROXY_CONTROL_FRAME_SIZE],
) -> Result<ProxyControlEvent, ProxyControlError> {
    validate_header(frame)?;
    if frame[CAPABILITY_OFFSET] != 0 {
        return Err(ProxyControlError::ReservedBytes);
    }
    if frame[LEDGER_GENERATION_RANGE].iter().any(|byte| *byte != 0) || decode_u64(frame, LEDGER_CLIENT_ID_RANGE) != 0 {
        return Err(ProxyControlError::ReservedBytes);
    }
    let generation = decode_generation(frame);
    validate_generation(generation)?;
    let grant_id = decode_u64(frame, GRANT_ID_RANGE);
    let detail = decode_u16(frame, DETAIL_RANGE);
    match frame[KIND_OFFSET] {
        READY if detail == 0 && grant_id == 0 => Ok(ProxyControlEvent::Ready { generation }),
        ADOPTED if detail == 0 && grant_id != 0 => Ok(ProxyControlEvent::Adopted { generation, grant_id }),
        REJECTED if grant_id != 0 => Ok(ProxyControlEvent::Rejected {
            generation,
            grant_id,
            reason: decode_rejection(detail)?,
        }),
        CLOSED if grant_id != 0 => Ok(ProxyControlEvent::Closed {
            generation,
            grant_id,
            reason: decode_close_reason(detail)?,
        }),
        STOPPED if detail == 0 && grant_id == 0 => Ok(ProxyControlEvent::Stopped { generation }),
        kind => Err(ProxyControlError::InvalidEvent(kind)),
    }
}

fn encode(
    kind: u8,
    capability: u8,
    detail: u16,
    generation: ProxyGeneration,
    grant_id: u64,
) -> [u8; PROXY_CONTROL_FRAME_SIZE] {
    let mut frame = [0; PROXY_CONTROL_FRAME_SIZE];
    frame[..2].copy_from_slice(&MAGIC);
    frame[VERSION_RANGE].copy_from_slice(&PROXY_CONTROL_VERSION.to_be_bytes());
    frame[KIND_OFFSET] = kind;
    frame[CAPABILITY_OFFSET] = capability;
    frame[DETAIL_RANGE].copy_from_slice(&detail.to_be_bytes());
    frame[GENERATION_RANGE].copy_from_slice(&generation.as_bytes());
    frame[GRANT_ID_RANGE].copy_from_slice(&grant_id.to_be_bytes());
    frame
}

fn validate_header(frame: &[u8; PROXY_CONTROL_FRAME_SIZE]) -> Result<(), ProxyControlError> {
    if frame[..2] != MAGIC {
        return Err(ProxyControlError::Magic);
    }
    let version = decode_u16(frame, VERSION_RANGE);
    if version != PROXY_CONTROL_VERSION {
        return Err(ProxyControlError::Version {
            expected: PROXY_CONTROL_VERSION,
            actual: version,
        });
    }
    Ok(())
}

fn validate_generation(generation: ProxyGeneration) -> Result<(), ProxyControlError> {
    if generation.is_zero() {
        Err(ProxyControlError::ZeroGeneration)
    } else {
        Ok(())
    }
}

fn decode_capability(value: u8) -> Result<ProxyCapability, ProxyControlError> {
    match value {
        1 => Ok(ProxyCapability::HttpTraffic),
        2 => Ok(ProxyCapability::DnsTraffic),
        3 => Ok(ProxyCapability::Upstream),
        4 => Ok(ProxyCapability::Credential),
        5 => Ok(ProxyCapability::Ledger),
        6 => Ok(ProxyCapability::PrivateNames),
        7 => Ok(ProxyCapability::Mcp),
        8 => Ok(ProxyCapability::Telemetry),
        9 => Ok(ProxyCapability::Policy),
        10 => Ok(ProxyCapability::TraceHints),
        value => Err(ProxyControlError::InvalidCapability(value)),
    }
}

const fn ledger_role_code(role: LedgerClientRole) -> u16 {
    match role {
        LedgerClientRole::VmOwner => 1,
        LedgerClientRole::Proxy => 2,
        LedgerClientRole::Coordinator => 3,
        LedgerClientRole::Reader => 4,
        LedgerClientRole::Maintainer => 5,
        LedgerClientRole::Supervisor => 6,
    }
}

fn decode_ledger_role(value: u16) -> Result<LedgerClientRole, ProxyControlError> {
    match value {
        1 => Ok(LedgerClientRole::VmOwner),
        2 => Ok(LedgerClientRole::Proxy),
        3 => Ok(LedgerClientRole::Coordinator),
        4 => Ok(LedgerClientRole::Reader),
        5 => Ok(LedgerClientRole::Maintainer),
        6 => Ok(LedgerClientRole::Supervisor),
        value => Err(ProxyControlError::InvalidLedgerRoleCode(value)),
    }
}

fn decode_rejection(value: u16) -> Result<ProxyControlRejection, ProxyControlError> {
    match value {
        1 => Ok(ProxyControlRejection::Capacity),
        2 => Ok(ProxyControlRejection::DuplicateCapability),
        3 => Ok(ProxyControlRejection::DuplicateGrant),
        4 => Ok(ProxyControlRejection::ShuttingDown),
        5 => Ok(ProxyControlRejection::NotReady),
        value => Err(ProxyControlError::InvalidRejection(value)),
    }
}

fn decode_close_reason(value: u16) -> Result<ProxyChannelCloseReason, ProxyControlError> {
    match value {
        1 => Ok(ProxyChannelCloseReason::Disconnected),
        2 => Ok(ProxyChannelCloseReason::ProtocolError),
        3 => Ok(ProxyChannelCloseReason::Revoked),
        value => Err(ProxyControlError::InvalidCloseReason(value)),
    }
}

fn decode_u16(frame: &[u8; PROXY_CONTROL_FRAME_SIZE], range: std::ops::Range<usize>) -> u16 {
    u16::from_be_bytes(frame[range].try_into().expect("fixed proxy control range"))
}

fn decode_u64(frame: &[u8; PROXY_CONTROL_FRAME_SIZE], range: std::ops::Range<usize>) -> u64 {
    u64::from_be_bytes(frame[range].try_into().expect("fixed proxy control range"))
}

fn decode_generation(frame: &[u8; PROXY_CONTROL_FRAME_SIZE]) -> ProxyGeneration {
    ProxyGeneration::new(
        frame[GENERATION_RANGE]
            .try_into()
            .expect("fixed proxy generation range"),
    )
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ProxyControlError {
    #[error("proxy control magic mismatch")]
    Magic,
    #[error("proxy control version mismatch: expected {expected}, got {actual}")]
    Version { expected: u16, actual: u16 },
    #[error("proxy control request kind {0} is invalid")]
    InvalidRequest(u8),
    #[error("proxy control event kind {0} is invalid")]
    InvalidEvent(u8),
    #[error("proxy capability {0} is invalid")]
    InvalidCapability(u8),
    #[error("proxy ledger capability is missing its exact ledger authority")]
    MissingLedgerAuthority,
    #[error("proxy ledger capability must carry the proxy role, got {0:?}")]
    InvalidLedgerRole(LedgerClientRole),
    #[error("proxy ledger role code {0} is invalid")]
    InvalidLedgerRoleCode(u16),
    #[error("proxy ledger authority is invalid: {0}")]
    LedgerAuthority(#[from] LedgerProtocolError),
    #[error("proxy control rejection {0} is invalid")]
    InvalidRejection(u16),
    #[error("proxy channel close reason {0} is invalid")]
    InvalidCloseReason(u16),
    #[error("proxy control reserved bytes are nonzero")]
    ReservedBytes,
    #[error("proxy generation must not be zero")]
    ZeroGeneration,
    #[error("proxy grant id must not be zero")]
    ZeroGrantId,
}

#[cfg(test)]
mod tests;
