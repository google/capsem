//! Build a `DnsEvent` row from the handler's structured result + the
//! envelope the agent sent (T3.3). Pure function -- testable without
//! sqlite. Callers (vsock dispatch in `capsem-process`) push the event
//! into the `DbWriter` channel via `WriteOp::DnsEvent`.
//!
//! There's no "DnsTelemetryHook" struct because DNS doesn't need the
//! chunk-pipeline machinery the MITM proxy uses -- a DNS query is
//! single-shot bytes-in / bytes-out. Keeping this as a free function
//! lets the dispatch decide when (and whether) to record, without
//! coupling the handler to a `DbWriter`.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::SystemTime;

use capsem_logger::events::DnsEvent;

use crate::net::dns::server::DnsHandlerResult;
use crate::security_engine::{DnsSecurityEvent, RuntimeSecurityEventType, SecurityEvent};

/// Build a `DnsEvent` row for one query.
///
/// `result.query` is `None` when the input bytes failed to decode at
/// all -- in that case we fall back to "INVALID_DNS_BYTES" / qtype=0
/// / qclass=0 so the row still surfaces in `dns_events` and ops can
/// see "the agent sent us garbage" without losing the timestamp +
/// trace_id correlation.
pub fn build_dns_event(
    result: &DnsHandlerResult,
    source_proto: Option<&str>,
    process_name: Option<String>,
    trace_id: Option<String>,
) -> DnsEvent {
    let (qname, qtype, qclass) = match &result.query {
        Some(q) => (q.qname.clone(), q.qtype, q.qclass),
        None => ("INVALID_DNS_BYTES".to_string(), 0u16, 0u16),
    };

    DnsEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        qname,
        qtype,
        qclass,
        rcode: result.rcode,
        answer_ip: first_answer_ip(&result.answer_bytes),
        decision: result.decision.as_str().to_string(),
        matched_rule: result.matched_rule.clone(),
        source_proto: source_proto.map(|s| s.to_string()),
        process_name,
        upstream_resolver_ms: result.upstream_resolver_ms,
        trace_id,
        policy_mode: result.policy_mode.clone(),
        policy_action: result.policy_action.clone(),
        policy_rule: result.policy_rule.clone(),
        policy_reason: result.policy_reason.clone(),
        credential_ref: None,
    }
}

fn first_answer_ip(packet: &[u8]) -> Option<String> {
    if packet.len() < 12 {
        return None;
    }
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let ancount = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let mut offset = 12usize;
    for _ in 0..qdcount {
        offset = skip_dns_name(packet, offset)?;
        offset = offset.checked_add(4)?;
        if offset > packet.len() {
            return None;
        }
    }
    for _ in 0..ancount {
        offset = skip_dns_name(packet, offset)?;
        if offset.checked_add(10)? > packet.len() {
            return None;
        }
        let rr_type = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let rdlen = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        if offset.checked_add(rdlen)? > packet.len() {
            return None;
        }
        match (rr_type, rdlen) {
            (1, 4) => {
                let addr = Ipv4Addr::new(
                    packet[offset],
                    packet[offset + 1],
                    packet[offset + 2],
                    packet[offset + 3],
                );
                return Some(addr.to_string());
            }
            (28, 16) => {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&packet[offset..offset + 16]);
                return Some(Ipv6Addr::from(octets).to_string());
            }
            _ => offset += rdlen,
        }
    }
    None
}

fn skip_dns_name(packet: &[u8], mut offset: usize) -> Option<usize> {
    let mut jumps = 0usize;
    loop {
        let len = *packet.get(offset)?;
        if len & 0b1100_0000 == 0b1100_0000 {
            packet.get(offset + 1)?;
            return Some(offset + 2);
        }
        if len == 0 {
            return Some(offset + 1);
        }
        if len & 0b1100_0000 != 0 {
            return None;
        }
        offset = offset.checked_add(1 + len as usize)?;
        if offset > packet.len() {
            return None;
        }
        jumps += 1;
        if jumps > 128 {
            return None;
        }
    }
}

pub fn security_event_from_dns_event(event: &DnsEvent) -> SecurityEvent {
    let security_event =
        SecurityEvent::new(RuntimeSecurityEventType::DnsQuery).with_dns(DnsSecurityEvent {
            qname: Some(event.qname.clone()),
            qtype: Some(event.qtype.to_string()),
        });
    match event.trace_id.clone() {
        Some(trace_id) => security_event.with_trace_id(trace_id),
        None => security_event,
    }
}

#[cfg(test)]
mod tests;
