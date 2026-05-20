//! End-to-end tests for the DNS handler + resolver, using a fake
//! UDP upstream bound on `127.0.0.1:0`. No system DNS, no internet.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

fn shared(p: PolicyConfig) -> super::server::SharedPolicy {
    Arc::new(tokio::sync::RwLock::new(Arc::new(p)))
}

use capsem_logger::events::Decision;
use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{Name, RData, Record, RecordType};
use tokio::net::UdpSocket;

use super::resolver::DnsResolver;
use super::server::DnsHandler;
use crate::net::policy::PolicyConfig;

fn build_query_bytes(name: &str, qtype: RecordType, id: u16) -> Vec<u8> {
    let mut msg = Message::new(id, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    let n = Name::from_ascii(name).unwrap();
    msg.add_query(Query::query(n, qtype));
    msg.to_vec().unwrap()
}

/// Spawn a fake DNS upstream that answers any A query with `answer_ip`
/// after an optional delay. Returns the bound socket address.
async fn spawn_fake_upstream(answer_ip: [u8; 4], delay: Duration) -> SocketAddr {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            let (n, peer) = match sock.recv_from(&mut buf).await {
                Ok(x) => x,
                Err(_) => break,
            };
            let req = Message::from_vec(&buf[..n]).unwrap();
            let mut resp = Message::new(req.metadata.id, MessageType::Response, OpCode::Query);
            resp.metadata.recursion_desired = req.metadata.recursion_desired;
            resp.metadata.recursion_available = true;
            resp.metadata.response_code = ResponseCode::NoError;
            for q in &req.queries {
                resp.add_query(q.clone());
                if q.query_type() == RecordType::A {
                    let rec = Record::from_rdata(
                        q.name().clone(),
                        60,
                        RData::A(Ipv4Addr::from(answer_ip).into()),
                    );
                    resp.add_answer(rec);
                }
            }
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let _ = sock.send_to(&resp.to_vec().unwrap(), peer).await;
        }
    });
    addr
}

/// Spawn a black-hole upstream that accepts queries but never replies.
/// Returns the bound socket address.
async fn spawn_blackhole_upstream() -> SocketAddr {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            if sock.recv_from(&mut buf).await.is_err() {
                break;
            }
            // Intentionally drop the query.
        }
    });
    addr
}

fn allow_all_policy() -> PolicyConfig {
    PolicyConfig::default()
}

fn policy_from_toml(toml: &str) -> Arc<tokio::sync::RwLock<Arc<PolicyConfig>>> {
    let policy = PolicyConfig::from_policy_toml_str(toml).expect("policy v2 TOML should parse");
    Arc::new(tokio::sync::RwLock::new(Arc::new(policy)))
}

fn block_specific_policy(domain: &str) -> PolicyConfig {
    let condition = if let Some(suffix) = domain.strip_prefix("*.") {
        format!(r#"qname.matches("^[^.]+\.{}$")"#, regex::escape(suffix))
    } else {
        format!(r#"qname == "{}""#, domain)
    };
    PolicyConfig::from_policy_toml_str(&format!(
        r#"
        [policy.dns.block_domain]
        on = "dns.query"
        if = '{condition}'
        decision = "block"
        priority = 1
        "#
    ))
    .unwrap()
}

fn policy_with_redirect(pattern: &str, qtype: Option<u16>, ips: Vec<IpAddr>) -> PolicyConfig {
    let mut clauses = vec![if let Some(suffix) = pattern.strip_prefix("*.") {
        format!(r#"qname.matches("^[^.]+\.{}$")"#, regex::escape(suffix))
    } else {
        format!(r#"qname == "{}""#, pattern)
    }];
    if let Some(qtype) = qtype {
        clauses.push(format!(r#"qtype == "{}""#, dns_qtype_name(qtype)));
    }
    let rewrite_value = ips
        .into_iter()
        .map(|ip| ip.to_string())
        .collect::<Vec<_>>()
        .join(",");
    PolicyConfig::from_policy_toml_str(&format!(
        r#"
        [policy.dns.rewrite_domain]
        on = "dns.query"
        if = '{}'
        decision = "rewrite"
        priority = 1
        rewrite_target = 'answer.ip =~ ".*"'
        rewrite_value = "{rewrite_value}"
        "#,
        clauses.join(" && ")
    ))
    .unwrap()
}

fn dns_qtype_name(qtype: u16) -> &'static str {
    match qtype {
        1 => "A",
        28 => "AAAA",
        _ => panic!("unexpected qtype in test: {qtype}"),
    }
}

mod policy_decisions;

mod resolver_behavior;

mod rewrite_behavior;

mod metrics_behavior;

mod cache_behavior;
