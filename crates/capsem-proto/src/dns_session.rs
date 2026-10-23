//! Cancellation is scoped to a nonzero correlation ID on its existing DNS stream.
use std::num::NonZeroU32;

pub use crate::{decode_dns_session_message, encode_dns_cancellation};
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

pub(crate) fn nonzero_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    NonZeroU32::deserialize(deserializer).map(NonZeroU32::get)
}

#[cfg(test)]
mod tests;
