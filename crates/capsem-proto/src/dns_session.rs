//! Cancellation is scoped to a nonzero correlation ID on its existing DNS stream.
use std::num::NonZeroU32;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DnsQueryCancellation {
    pub cancel_query_id: NonZeroU32,
}

/// The externally tagged MessagePack enum dispatches in one pass without
/// buffering a serde Content tree. Encoding may borrow the original query.
#[derive(Debug, Serialize, Deserialize)]
pub enum DnsSessionMessage<Q = crate::DnsRequest> {
    Cancel(DnsQueryCancellation),
    Query(Q),
}

pub fn decode_dns_session_message(payload: &[u8]) -> Result<DnsSessionMessage> {
    rmp_serde::from_slice(payload).context("failed to decode DNS session message")
}

pub fn encode_dns_cancellation(cancel: &DnsQueryCancellation) -> Result<Vec<u8>> {
    let message = DnsSessionMessage::<&crate::DnsRequest>::Cancel(*cancel);
    let payload = rmp_serde::to_vec_named(&message).context("failed to encode DNS cancellation")?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

#[cfg(test)]
mod tests;
