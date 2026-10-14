//! Fixed records for coordinator-granted gateway Unix connections.
//!
//! The confined gateway never names a socket path. It may ask for a fresh
//! service connection, or name a VM whose current owner handoff endpoint the
//! coordinator derives and authenticates from its registry.

use anyhow::{bail, Result};

pub const GATEWAY_GRANT_FRAME_SIZE: usize = 80;
pub const GATEWAY_GRANT_MAX_FDS: usize = 1;
pub const MAX_GATEWAY_VM_ID_BYTES: usize = 64;

const MAGIC: [u8; 2] = *b"GG";
const VERSION: u8 = 1;
const KIND_OFFSET: usize = 3;
const REQUEST_ID_RANGE: std::ops::Range<usize> = 4..12;
const DETAIL_OFFSET: usize = 12;
const VM_ID_LENGTH_OFFSET: usize = 13;
const VM_ID_RANGE: std::ops::Range<usize> = 14..14 + MAX_GATEWAY_VM_ID_BYTES;
const RESERVED_RANGE: std::ops::Range<usize> = VM_ID_RANGE.end..GATEWAY_GRANT_FRAME_SIZE;

const OPEN_SERVICE: u8 = 1;
const OPEN_OWNER_HANDOFF: u8 = 2;
const GRANTED: u8 = 101;
const DENIED: u8 = 102;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GatewayGrantRequest {
    OpenService { request_id: u64 },
    OpenOwnerHandoff { request_id: u64, vm_id: String },
}

impl GatewayGrantRequest {
    pub fn request_id(&self) -> u64 {
        match self {
            Self::OpenService { request_id } | Self::OpenOwnerHandoff { request_id, .. } => *request_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayGrantKind {
    Service,
    OwnerHandoff,
}

impl GatewayGrantKind {
    fn code(self) -> u8 {
        match self {
            Self::Service => OPEN_SERVICE,
            Self::OwnerHandoff => OPEN_OWNER_HANDOFF,
        }
    }

    fn decode(code: u8) -> Result<Self> {
        match code {
            OPEN_SERVICE => Ok(Self::Service),
            OPEN_OWNER_HANDOFF => Ok(Self::OwnerHandoff),
            _ => bail!("invalid gateway grant kind {code}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayGrantDenial {
    Unavailable,
    NotFound,
    Revoked,
    InvalidRequest,
}

impl GatewayGrantDenial {
    fn code(self) -> u8 {
        match self {
            Self::Unavailable => 1,
            Self::NotFound => 2,
            Self::Revoked => 3,
            Self::InvalidRequest => 4,
        }
    }

    fn decode(code: u8) -> Result<Self> {
        match code {
            1 => Ok(Self::Unavailable),
            2 => Ok(Self::NotFound),
            3 => Ok(Self::Revoked),
            4 => Ok(Self::InvalidRequest),
            _ => bail!("invalid gateway grant denial {code}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayGrantResponse {
    Granted {
        request_id: u64,
        kind: GatewayGrantKind,
    },
    Denied {
        request_id: u64,
        reason: GatewayGrantDenial,
    },
}

impl GatewayGrantResponse {
    pub fn request_id(self) -> u64 {
        match self {
            Self::Granted { request_id, .. } | Self::Denied { request_id, .. } => request_id,
        }
    }
}

pub fn encode_gateway_grant_request(request: &GatewayGrantRequest) -> Result<[u8; GATEWAY_GRANT_FRAME_SIZE]> {
    let mut frame = base(match request {
        GatewayGrantRequest::OpenService { .. } => OPEN_SERVICE,
        GatewayGrantRequest::OpenOwnerHandoff { .. } => OPEN_OWNER_HANDOFF,
    });
    frame[REQUEST_ID_RANGE].copy_from_slice(&request.request_id().to_be_bytes());
    if let GatewayGrantRequest::OpenOwnerHandoff { vm_id, .. } = request {
        let id = vm_id.as_bytes();
        if id.is_empty() || id.len() > MAX_GATEWAY_VM_ID_BYTES {
            bail!("gateway VM id length is out of bounds");
        }
        frame[VM_ID_LENGTH_OFFSET] = id.len() as u8;
        frame[VM_ID_RANGE.start..VM_ID_RANGE.start + id.len()].copy_from_slice(id);
    }
    Ok(frame)
}

pub fn decode_gateway_grant_request(frame: &[u8; GATEWAY_GRANT_FRAME_SIZE]) -> Result<GatewayGrantRequest> {
    validate(frame)?;
    let request_id = u64::from_be_bytes(frame[REQUEST_ID_RANGE].try_into().unwrap());
    match frame[KIND_OFFSET] {
        OPEN_SERVICE if frame[VM_ID_LENGTH_OFFSET] == 0 => Ok(GatewayGrantRequest::OpenService { request_id }),
        OPEN_OWNER_HANDOFF => {
            let length = usize::from(frame[VM_ID_LENGTH_OFFSET]);
            if length == 0 || length > MAX_GATEWAY_VM_ID_BYTES {
                bail!("gateway VM id length is out of bounds");
            }
            if frame[VM_ID_RANGE.start + length..VM_ID_RANGE.end]
                .iter()
                .any(|byte| *byte != 0)
            {
                bail!("gateway grant frame has nonzero VM id padding");
            }
            let vm_id = std::str::from_utf8(&frame[VM_ID_RANGE.start..VM_ID_RANGE.start + length])?.to_owned();
            Ok(GatewayGrantRequest::OpenOwnerHandoff { request_id, vm_id })
        }
        kind => bail!("invalid gateway grant request kind {kind}"),
    }
}

pub fn encode_gateway_grant_response(response: GatewayGrantResponse) -> [u8; GATEWAY_GRANT_FRAME_SIZE] {
    let (kind, request_id, detail) = match response {
        GatewayGrantResponse::Granted { request_id, kind } => (GRANTED, request_id, kind.code()),
        GatewayGrantResponse::Denied { request_id, reason } => (DENIED, request_id, reason.code()),
    };
    let mut frame = base(kind);
    frame[REQUEST_ID_RANGE].copy_from_slice(&request_id.to_be_bytes());
    frame[DETAIL_OFFSET] = detail;
    frame
}

pub fn decode_gateway_grant_response(frame: &[u8; GATEWAY_GRANT_FRAME_SIZE]) -> Result<GatewayGrantResponse> {
    validate(frame)?;
    if frame[VM_ID_LENGTH_OFFSET] != 0 || frame[VM_ID_RANGE].iter().any(|byte| *byte != 0) {
        bail!("gateway grant response has unexpected payload");
    }
    let request_id = u64::from_be_bytes(frame[REQUEST_ID_RANGE].try_into().unwrap());
    match frame[KIND_OFFSET] {
        GRANTED => Ok(GatewayGrantResponse::Granted {
            request_id,
            kind: GatewayGrantKind::decode(frame[DETAIL_OFFSET])?,
        }),
        DENIED => Ok(GatewayGrantResponse::Denied {
            request_id,
            reason: GatewayGrantDenial::decode(frame[DETAIL_OFFSET])?,
        }),
        kind => bail!("invalid gateway grant response kind {kind}"),
    }
}

fn base(kind: u8) -> [u8; GATEWAY_GRANT_FRAME_SIZE] {
    let mut frame = [0; GATEWAY_GRANT_FRAME_SIZE];
    frame[..2].copy_from_slice(&MAGIC);
    frame[2] = VERSION;
    frame[KIND_OFFSET] = kind;
    frame
}

fn validate(frame: &[u8; GATEWAY_GRANT_FRAME_SIZE]) -> Result<()> {
    if frame[..2] != MAGIC {
        bail!("invalid gateway grant magic");
    }
    if frame[2] != VERSION {
        bail!("unsupported gateway grant version {}", frame[2]);
    }
    if frame[RESERVED_RANGE].iter().any(|byte| *byte != 0) {
        bail!("gateway grant reserved bytes are nonzero");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
