//! DNS wire-format parser. Wraps `hickory-proto` Message decode/encode
//! with the small surface the capsem DNS proxy (T3) actually uses:
//! parse a query, build an NXDOMAIN response, build a SERVFAIL response.
//!
//! Bytes go in, structured `DnsQuery` comes out for policy + telemetry;
//! synthetic responses go out as bytes ready to ship over the vsock
//! envelope. Forwarded upstream answers never round-trip through this
//! module -- they're returned verbatim so resolver-specific record
//! shapes (CNAMEs, TTLs, EDNS) survive untouched.
//!
//! The capsem-agent guest crate does NOT depend on hickory-proto: the
//! agent forwards raw DNS bytes to the host, where this module decodes
//! them. Keeps the musl cross-compile surface small.
//!
//! No `tracing` calls here -- the parser is a pure function. Callers
//! (the DNS handler) do the policy / telemetry observability.

use std::net::IpAddr;

use anyhow::{anyhow, Context, Result};
use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::{rdata, RData, Record, RecordType};

/// A single parsed DNS question -- the operationally relevant slice of
/// a DNS query message. Multi-question queries are technically allowed
/// by RFC 1035 but rejected by every modern resolver; we surface the
/// first question and report the rest via `extra_questions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsQuery {
    /// Transaction id from the DNS header (used to match responses).
    pub id: u16,
    /// Query hostname, lowercased, with the trailing root dot stripped
    /// (e.g. "anthropic.com" not "anthropic.com.").
    pub qname: String,
    /// Query type as a u16 (1 = A, 28 = AAAA, 16 = TXT, 15 = MX, ...).
    /// We don't expose hickory's `RecordType` to keep the public type
    /// free of hickory-specific dependencies for downstream consumers.
    pub qtype: u16,
    /// Query class as a u16 (almost always 1 = IN).
    pub qclass: u16,
    /// Number of additional questions in the message beyond the first.
    /// Real DNS queries are always single-question; multi-question is
    /// usually a malformed or adversarial input.
    pub extra_questions: usize,
}

/// Decode the first question of a wire-format DNS query message.
///
/// Returns Err if the bytes don't decode as a DNS message at all, or if
/// the message contains zero questions. Multi-question messages are
/// accepted (with `extra_questions` recording the count) so the handler
/// can log them; policy / telemetry decide what to do.
pub fn parse_query(bytes: &[u8]) -> Result<DnsQuery> {
    let msg = Message::from_vec(bytes).context("failed to decode DNS message")?;
    let first = msg
        .queries
        .first()
        .ok_or_else(|| anyhow!("DNS message has no questions"))?;

    let qname = first.name().to_ascii();
    // hickory's `to_ascii` returns the FQDN with a trailing dot. Strip
    // it so policy comparison ("anthropic.com") matches the way every
    // operator writes domains.
    let qname = qname.trim_end_matches('.').to_lowercase();

    Ok(DnsQuery {
        id: msg.metadata.id,
        qname,
        qtype: u16::from(first.query_type()),
        qclass: u16::from(first.query_class()),
        extra_questions: msg.queries.len().saturating_sub(1),
    })
}

/// Build a synthetic NXDOMAIN response for a query message.
///
/// Used on the policy-blocked path: the guest tries to resolve a domain
/// that's fully denied, and we want their resolver to fail fast with
/// "name does not exist" rather than time out. The response preserves
/// the original transaction id and questions, sets the response flag,
/// recursion-available, and `ResponseCode::NXDomain`.
pub fn build_nxdomain(query_bytes: &[u8]) -> Result<Vec<u8>> {
    build_synthetic_response(query_bytes, ResponseCode::NXDomain)
}

/// Build a synthetic SERVFAIL response for a query message.
///
/// Used when the upstream resolver fails (timeout, all upstreams
/// unreachable, network error). The guest's resolver retries or
/// returns "temporary failure" rather than caching a NXDOMAIN.
pub fn build_servfail(query_bytes: &[u8]) -> Result<Vec<u8>> {
    build_synthetic_response(query_bytes, ResponseCode::ServFail)
}

/// Build a synthetic NoError response with one or more A/AAAA answer
/// records (T3.d). Used by the policy-redirect path: an admin
/// configures `DnsRedirect { qname, qtype, answers, ttl }` and we
/// synthesize the response locally instead of forwarding upstream.
///
/// Filtering: only IPs whose family matches the query's qtype are
/// included as answer records. An IPv4 in the answer list with an
/// AAAA query is silently skipped (and vice versa) -- that's the
/// standard "name exists, no record of that type" DNS shape, and
/// it's what an admin who set `qtype = None` with mixed answers
/// would expect: A queries get the IPv4s, AAAA queries get the
/// IPv6s, no cross-family fabrication.
///
/// `ttl` flows verbatim into every answer record. Callers should
/// keep it short (~60s) so guest resolvers re-query promptly when
/// the redirect rule changes.
pub fn build_redirect_response(
    query_bytes: &[u8],
    answers: &[IpAddr],
    ttl: u32,
) -> Result<Vec<u8>> {
    let request = Message::from_vec(query_bytes).context("failed to decode DNS message")?;
    let qtype = request
        .queries
        .first()
        .ok_or_else(|| anyhow!("DNS message has no questions"))?
        .query_type();

    let mut response = Message::new(request.metadata.id, MessageType::Response, OpCode::Query);
    response.metadata.recursion_desired = request.metadata.recursion_desired;
    response.metadata.recursion_available = true;
    response.metadata.response_code = ResponseCode::NoError;
    response.add_queries(request.queries.iter().cloned());

    let qname = response.queries[0].name().clone();
    for ip in answers {
        let rdata_opt = match (qtype, ip) {
            (RecordType::A, IpAddr::V4(v4)) => Some(RData::A(rdata::A(*v4))),
            (RecordType::AAAA, IpAddr::V6(v6)) => Some(RData::AAAA(rdata::AAAA(*v6))),
            // Cross-family: skip silently (NoError + zero matching
            // answers means "name exists, no record of that type").
            _ => None,
        };
        if let Some(rdata) = rdata_opt {
            response.add_answer(Record::from_rdata(qname.clone(), ttl, rdata));
        }
    }

    response
        .to_vec()
        .context("failed to encode synthetic DNS redirect response")
}

fn build_synthetic_response(query_bytes: &[u8], rcode: ResponseCode) -> Result<Vec<u8>> {
    let request = Message::from_vec(query_bytes).context("failed to decode DNS message")?;
    let mut response = Message::new(request.metadata.id, MessageType::Response, OpCode::Query);
    response.metadata.recursion_desired = request.metadata.recursion_desired;
    response.metadata.recursion_available = true;
    response.metadata.response_code = rcode;
    response.add_queries(request.queries.iter().cloned());
    response
        .to_vec()
        .context("failed to encode synthetic DNS response")
}

#[cfg(test)]
mod proptests;
#[cfg(test)]
mod tests;
