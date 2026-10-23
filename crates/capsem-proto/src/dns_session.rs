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

#[derive(Debug, Serialize, Deserialize)]
pub enum DnsSessionReply<R = crate::DnsResponse> {
    Answer(R),
    Pending {
        id: NonZeroU32,
        registration: crate::approval::ApprovalRegistration,
    },
}

pub(crate) fn nonzero_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    NonZeroU32::deserialize(deserializer).map(NonZeroU32::get)
}

/// One DNS query forwarded from the guest agent to the host DNS handler.
///
/// `raw` is the wire-format DNS query bytes -- the agent never parses
/// them, so the host (which already speaks DNS via `hickory-proto`)
/// stays the single decoder. `proto` lets the host distinguish UDP
/// queries (datagram, single-shot) from TCP queries (length-prefixed
/// stream, may carry larger answers like AXFR / TXT) so the response
/// can match. `process_name` is reserved for T3.3 telemetry --
/// looking up the source process for a UDP DNS query at agent time
/// is racy (transient sockets), so we ship `None` for now and let the
/// host fold it in later if `/proc/net/udp` correlation becomes
/// reliable enough to bother.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DnsRequest {
    /// Correlation id chosen by the guest. The host echoes it in the
    /// matching `DnsResponse` so many queries can be in flight on one
    /// vsock connection and be answered out of order. Zero and missing IDs
    /// are refused; every query must support scoped cancellation.
    #[serde(deserialize_with = "nonzero_id")]
    pub id: u32,
    #[serde(with = "serde_bytes")]
    pub raw: Vec<u8>,
    /// "udp" or "tcp" -- the source-side transport, NOT the path used
    /// to reach the upstream nameserver (which is always UDP today).
    pub proto: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
}

/// One DNS answer flowing host -> guest agent over the DNS vsock port.
///
/// `raw` is the wire-format response (synthetic NXDOMAIN, synthetic
/// SERVFAIL, or upstream-forwarded answer). `decision` mirrors
/// `capsem_logger::events::Decision::as_str()` ("allowed", "denied",
/// "error") so the agent can log the outcome without depending on the
/// logger crate. `rcode` is the DNS response code (0 / 2 / 3) so a
/// future agent-side metric can distinguish NXDOMAIN-on-block from
/// NXDOMAIN-from-upstream without re-parsing the wire bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DnsResponse {
    /// The `DnsRequest::id` this answers.
    #[serde(deserialize_with = "nonzero_id")]
    pub id: u32,
    #[serde(with = "serde_bytes")]
    pub raw: Vec<u8>,
    pub decision: String,
    pub rcode: u16,
}

#[cfg(test)]
mod tests;
