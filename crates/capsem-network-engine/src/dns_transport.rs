use capsem_logger::events::Decision;

use crate::dns_parser::DnsQuery;

/// Result of handling one DNS query. The answer bytes are always populated on
/// transport paths that should answer the guest. Malformed input uses
/// `query = None` and an empty answer so callers can drop the request while
/// still writing a structured telemetry row.
#[derive(Debug, Clone)]
pub struct DnsHandlerResult {
    /// Wire-format DNS response, ready to ship over the vsock envelope.
    pub answer_bytes: Vec<u8>,
    /// Parsed query metadata. `None` on malformed input where raw bytes did
    /// not decode.
    pub query: Option<DnsQuery>,
    /// Resolver or runtime policy outcome.
    pub decision: Decision,
    /// Matched policy/rule label for legacy DNS event projection.
    pub matched_rule: Option<String>,
    /// Wall time of the upstream resolve attempt, in milliseconds.
    pub upstream_resolver_ms: u64,
    /// DNS rcode for the answer.
    pub rcode: u16,
    /// Policy engine mode that produced this decision, if any.
    pub policy_mode: Option<String>,
    /// Typed policy action when policy matched.
    pub policy_action: Option<String>,
    /// Fully qualified policy rule id.
    pub policy_rule: Option<String>,
    /// Human-readable policy reason or fail-closed detail.
    pub policy_reason: Option<String>,
}

impl DnsHandlerResult {
    pub fn allowed(answer_bytes: Vec<u8>, query: DnsQuery, upstream_ms: u64, rcode: u16) -> Self {
        Self {
            answer_bytes,
            query: Some(query),
            decision: Decision::Allowed,
            matched_rule: None,
            upstream_resolver_ms: upstream_ms,
            rcode,
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
        }
    }

    pub fn upstream_failed(answer_bytes: Vec<u8>, query: DnsQuery, upstream_ms: u64) -> Self {
        Self {
            answer_bytes,
            query: Some(query),
            decision: Decision::Error,
            matched_rule: None,
            upstream_resolver_ms: upstream_ms,
            rcode: 2,
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
        }
    }

    pub fn parse_failed() -> Self {
        Self {
            answer_bytes: Vec::new(),
            query: None,
            decision: Decision::Error,
            matched_rule: None,
            upstream_resolver_ms: 0,
            rcode: 1,
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
        }
    }
}
