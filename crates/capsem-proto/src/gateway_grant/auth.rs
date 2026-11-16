//! Bounded authentication records on the existing inherited gateway channel.
//! Secrets are existing operator/managed capabilities, never caller actor tags.
use anyhow::{bail, Result};
use capsem_crypto::secrets::{constant_time_eq, Zeroizing};

use super::GATEWAY_GRANT_FRAME_SIZE;

/// Fixed secret proof bytes, with audited erasure and constant-time equality.
#[derive(Clone)]
pub struct GatewayAuthSecret<const N: usize>(Zeroizing<[u8; N]>);

impl<const N: usize> GatewayAuthSecret<N> {
    pub fn from_bytes(bytes: [u8; N]) -> Self {
        Self(Zeroizing::new(bytes))
    }
    pub fn as_bytes(&self) -> &[u8; N] {
        &self.0
    }
}
impl<const N: usize> std::fmt::Debug for GatewayAuthSecret<N> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GatewayAuthSecret([REDACTED])")
    }
}
impl<const N: usize> PartialEq for GatewayAuthSecret<N> {
    fn eq(&self, other: &Self) -> bool {
        constant_time_eq(self.as_bytes(), other.as_bytes())
    }
}
impl<const N: usize> Eq for GatewayAuthSecret<N> {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GatewayAuthRequest {
    RegisterOperator {
        request_id: u64,
        token: GatewayAuthSecret<64>,
    },
    AuthenticateAgent {
        request_id: u64,
        access_id: [u8; 16],
        capability: GatewayAuthSecret<32>,
    },
}
impl GatewayAuthRequest {
    pub fn request_id(&self) -> u64 {
        match self {
            Self::RegisterOperator { request_id, .. } | Self::AuthenticateAgent { request_id, .. } => *request_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewayAuthDenial {
    Forbidden,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewayAuthResponse {
    RegisteredOperator { request_id: u64 },
    AuthenticatedAgent { request_id: u64, access_id: [u8; 16] },
    Denied { request_id: u64, reason: GatewayAuthDenial },
}
impl GatewayAuthResponse {
    pub fn request_id(self) -> u64 {
        match self {
            Self::RegisteredOperator { request_id }
            | Self::AuthenticatedAgent { request_id, .. }
            | Self::Denied { request_id, .. } => request_id,
        }
    }
}

const MAGIC: [u8; 2] = *b"GA";
const VERSION: u8 = 1;
const DATA_OFFSET: usize = 14;
const MAX_DATA: usize = 64;
const REGISTER_OPERATOR: u8 = 1;
const AUTHENTICATE_AGENT: u8 = 2;
const REGISTERED_OPERATOR: u8 = 101;
const AUTHENTICATED_AGENT: u8 = 102;
const DENIED: u8 = 103;

/// Discriminator only: the selected decoder must still validate every byte.
pub fn is_gateway_auth_frame(frame: &[u8; GATEWAY_GRANT_FRAME_SIZE]) -> bool {
    frame[..2] == MAGIC
}

pub fn encode_gateway_auth_request(request: &GatewayAuthRequest) -> Result<[u8; GATEWAY_GRANT_FRAME_SIZE]> {
    match request {
        GatewayAuthRequest::RegisterOperator { request_id, token } => {
            operator_token(token.as_bytes())?;
            frame(REGISTER_OPERATOR, *request_id, 0, token.as_bytes())
        }
        GatewayAuthRequest::AuthenticateAgent {
            request_id,
            access_id,
            capability,
        } => {
            access_identity(access_id)?;
            let mut bytes = Zeroizing::new([0; 48]);
            bytes[..16].copy_from_slice(access_id);
            bytes[16..].copy_from_slice(capability.as_bytes());
            frame(AUTHENTICATE_AGENT, *request_id, 0, bytes.as_ref())
        }
    }
}

pub fn decode_gateway_auth_request(frame: &[u8; GATEWAY_GRANT_FRAME_SIZE]) -> Result<GatewayAuthRequest> {
    let (kind, request_id, detail, payload) = parse(frame)?;
    if detail != 0 {
        bail!("gateway authentication request has unexpected detail");
    }
    match (kind, payload.len()) {
        (REGISTER_OPERATOR, 64) => {
            let token: [u8; 64] = payload.try_into().unwrap();
            operator_token(&token)?;
            Ok(GatewayAuthRequest::RegisterOperator {
                request_id,
                token: GatewayAuthSecret::from_bytes(token),
            })
        }
        (AUTHENTICATE_AGENT, 48) => {
            let access_id = payload[..16].try_into().unwrap();
            access_identity(&access_id)?;
            Ok(GatewayAuthRequest::AuthenticateAgent {
                request_id,
                access_id,
                capability: GatewayAuthSecret::from_bytes(payload[16..].try_into().unwrap()),
            })
        }
        _ => bail!("invalid gateway authentication request kind or payload"),
    }
}

pub fn encode_gateway_auth_response(response: GatewayAuthResponse) -> Result<[u8; GATEWAY_GRANT_FRAME_SIZE]> {
    match response {
        GatewayAuthResponse::RegisteredOperator { request_id } => frame(REGISTERED_OPERATOR, request_id, 0, &[]),
        GatewayAuthResponse::AuthenticatedAgent { request_id, access_id } => {
            access_identity(&access_id)?;
            frame(AUTHENTICATED_AGENT, request_id, 0, &access_id)
        }
        GatewayAuthResponse::Denied { request_id, reason } => frame(
            DENIED,
            request_id,
            match reason {
                GatewayAuthDenial::Forbidden => 1,
                GatewayAuthDenial::Unavailable => 2,
            },
            &[],
        ),
    }
}

pub fn decode_gateway_auth_response(frame: &[u8; GATEWAY_GRANT_FRAME_SIZE]) -> Result<GatewayAuthResponse> {
    let (kind, request_id, detail, payload) = parse(frame)?;
    match (kind, detail, payload.len()) {
        (REGISTERED_OPERATOR, 0, 0) => Ok(GatewayAuthResponse::RegisteredOperator { request_id }),
        (AUTHENTICATED_AGENT, 0, 16) => {
            let access_id = payload.try_into().unwrap();
            access_identity(&access_id)?;
            Ok(GatewayAuthResponse::AuthenticatedAgent { request_id, access_id })
        }
        (DENIED, reason @ (1 | 2), 0) => Ok(GatewayAuthResponse::Denied {
            request_id,
            reason: if reason == 1 {
                GatewayAuthDenial::Forbidden
            } else {
                GatewayAuthDenial::Unavailable
            },
        }),
        _ => bail!("invalid gateway authentication response kind or payload"),
    }
}

fn frame(kind: u8, request_id: u64, detail: u8, payload: &[u8]) -> Result<[u8; GATEWAY_GRANT_FRAME_SIZE]> {
    if request_id == 0 || payload.len() > MAX_DATA {
        bail!("invalid gateway authentication identity or length");
    }
    let mut frame = [0; GATEWAY_GRANT_FRAME_SIZE];
    frame[..2].copy_from_slice(&MAGIC);
    frame[2] = VERSION;
    frame[3] = kind;
    frame[4..12].copy_from_slice(&request_id.to_be_bytes());
    frame[12] = detail;
    frame[13] = payload.len() as u8;
    frame[DATA_OFFSET..DATA_OFFSET + payload.len()].copy_from_slice(payload);
    Ok(frame)
}

fn parse(frame: &[u8; GATEWAY_GRANT_FRAME_SIZE]) -> Result<(u8, u64, u8, &[u8])> {
    if frame[..2] != MAGIC || frame[2] != VERSION {
        bail!("invalid gateway authentication magic or version");
    }
    let request_id = u64::from_be_bytes(frame[4..12].try_into().unwrap());
    let length = usize::from(frame[13]);
    if request_id == 0 || length > MAX_DATA {
        bail!("invalid gateway authentication identity or length");
    }
    if frame[DATA_OFFSET + length..].iter().any(|byte| *byte != 0) {
        bail!("gateway authentication padding is nonzero");
    }
    Ok((
        frame[3],
        request_id,
        frame[12],
        &frame[DATA_OFFSET..DATA_OFFSET + length],
    ))
}

fn access_identity(id: &[u8; 16]) -> Result<()> {
    if id[6] >> 4 != 4 || id[8] & 0xc0 != 0x80 {
        bail!("gateway authentication requires a UUIDv4 access identity");
    }
    Ok(())
}

fn operator_token(token: &[u8; 64]) -> Result<()> {
    if !token.iter().all(u8::is_ascii_alphanumeric) {
        bail!("invalid gateway operator credential format");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
