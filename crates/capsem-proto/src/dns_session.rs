//! Cancellation is scoped to a nonzero correlation ID on its existing DNS stream.
use std::num::NonZeroU32;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DnsQueryCancellation {
    pub cancel_query_id: NonZeroU32,
}

/// Query frames retain their original representation; cancellation cannot be
/// mistaken for a query by an older host and fails closed there.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum DnsSessionMessage {
    Cancel(DnsQueryCancellation),
    Query(crate::DnsRequest),
}

pub fn decode_dns_session_message(payload: &[u8]) -> Result<DnsSessionMessage> {
    rmp_serde::from_slice(payload).context("failed to decode DNS session message")
}

pub fn encode_dns_cancellation(cancel: &DnsQueryCancellation) -> Result<Vec<u8>> {
    let payload = rmp_serde::to_vec_named(cancel).context("failed to encode DNS cancellation")?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

#[cfg(test)]
mod tests;
