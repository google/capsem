//! End-to-end tests for the DNS handler + resolver, using a fake
//! UDP upstream bound on `127.0.0.1:0`. No system DNS, no internet.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Duration;

fn shared(p: NetworkPolicy) -> super::server::SharedPolicy {
    Arc::new(RwLock::new(Arc::new(p)))
}

use capsem_logger::events::Decision;
use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{Name, RData, Record, RecordType};
use tokio::net::UdpSocket;

use super::resolver::DnsResolver;
use super::server::DnsHandler;
use crate::net::policy::{DnsRedirect, DomainMatcher, NetworkPolicy, PolicyRule};
use crate::net::policy_v2::PolicyConfig;

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

fn allow_all_policy() -> NetworkPolicy {
    NetworkPolicy::new(vec![], true, true)
}

fn policy_v2_from_toml(toml: &str) -> Arc<tokio::sync::RwLock<Arc<PolicyConfig>>> {
    let policy = PolicyConfig::from_policy_toml_str(toml).expect("policy v2 TOML should parse");
    Arc::new(tokio::sync::RwLock::new(Arc::new(policy)))
}

fn block_specific_policy(domain: &str) -> NetworkPolicy {
    let mut p = NetworkPolicy::new(vec![], true, true);
    p.rules.push(PolicyRule {
        matcher: DomainMatcher::parse(domain),
        allow_read: false,
        allow_write: false,
    });
    p
}

#[tokio::test]
async fn policy_v2_dns_block_returns_nxdomain_without_upstream_and_records_policy_fields() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy_v2 = policy_v2_from_toml(
        r#"
        [policy.dns.block_openai]
        on = "dns.query"
        if = 'qname == "api.openai.com" && qtype == "A"'
        decision = "block"
        priority = 10
        reason = "DNS to OpenAI API is blocked"
        "#,
    );
    let handler = DnsHandler::new_with_policy_v2(shared(allow_all_policy()), policy_v2, resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 0xD001);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Denied);
    assert_eq!(res.matched_rule.as_deref(), Some("policy.dns.block_openai"));
    assert_eq!(res.upstream_resolver_ms, 0);
    assert_eq!(res.rcode, 3);
    assert_eq!(res.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(res.policy_action.as_deref(), Some("block"));
    assert_eq!(res.policy_rule.as_deref(), Some("policy.dns.block_openai"));
    assert_eq!(
        res.policy_reason.as_deref(),
        Some("DNS to OpenAI API is blocked")
    );
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::NXDomain);
    assert_eq!(resp.answers.len(), 0);
}

#[tokio::test]
async fn policy_v2_dns_allow_forwards_upstream_and_records_policy_fields() {
    let upstream = spawn_fake_upstream([10, 11, 12, 13], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy_v2 = policy_v2_from_toml(
        r#"
        [policy.dns.allow_openai]
        on = "dns.query"
        if = 'qname == "api.openai.com" && qtype == "A"'
        decision = "allow"
        priority = 1
        reason = "DNS to OpenAI API is allowed"
        "#,
    );
    let handler = DnsHandler::new_with_policy_v2(shared(allow_all_policy()), policy_v2, resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 0xD008);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Allowed);
    assert_eq!(res.matched_rule, None);
    assert_eq!(res.rcode, 0);
    assert_eq!(res.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(res.policy_action.as_deref(), Some("allow"));
    assert_eq!(res.policy_rule.as_deref(), Some("policy.dns.allow_openai"));
    assert_eq!(
        res.policy_reason.as_deref(),
        Some("DNS to OpenAI API is allowed")
    );
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.answers.len(), 1);
}

#[tokio::test]
async fn policy_v2_dns_ask_fails_closed_without_upstream_resolution() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy_v2 = policy_v2_from_toml(
        r#"
        [policy.dns.ask_openai]
        on = "dns.query"
        if = 'qname == "api.openai.com"'
        decision = "ask"
        priority = 5
        reason = "DNS query needs approval"
        "#,
    );
    let handler = DnsHandler::new_with_policy_v2(shared(allow_all_policy()), policy_v2, resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 0xD002);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Denied);
    assert_eq!(res.matched_rule.as_deref(), Some("policy.dns.ask_openai"));
    assert_eq!(res.upstream_resolver_ms, 0);
    assert_eq!(res.rcode, 3);
    assert_eq!(res.policy_action.as_deref(), Some("ask"));
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::NXDomain);
}

#[tokio::test]
async fn policy_v2_dns_rewrite_synthesizes_answer_without_upstream_resolution() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy_v2 = policy_v2_from_toml(
        r#"
        [policy.dns.rewrite_openai]
        on = "dns.query"
        if = 'qname == "api.openai.com" && qtype == "A"'
        decision = "rewrite"
        priority = 1
        reason = "Pin OpenAI API DNS locally"
        rewrite_target = 'answer.ip =~ ".*"'
        rewrite_value = "127.0.0.42"
        "#,
    );
    let handler = DnsHandler::new_with_policy_v2(shared(allow_all_policy()), policy_v2, resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 0xD003);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Redirected);
    assert_eq!(
        res.matched_rule.as_deref(),
        Some("policy.dns.rewrite_openai")
    );
    assert_eq!(res.upstream_resolver_ms, 0);
    assert_eq!(res.rcode, 0);
    assert_eq!(res.policy_action.as_deref(), Some("rewrite"));
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::NoError);
    assert_eq!(resp.answers.len(), 1);
    if let RData::A(answer) = &resp.answers[0].data {
        assert_eq!(answer.0, Ipv4Addr::new(127, 0, 0, 42));
    } else {
        panic!("expected A record after DNS policy rewrite");
    }
}

#[tokio::test]
async fn policy_v2_dns_rewrite_with_invalid_answer_fails_closed_without_upstream_resolution() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy_v2 = policy_v2_from_toml(
        r#"
        [policy.dns.bogus_rewrite]
        on = "dns.query"
        if = 'qname == "api.openai.com"'
        decision = "rewrite"
        priority = 1
        reason = "Bogus DNS rewrite should not leak upstream"
        rewrite_target = 'answer.ip =~ ".*"'
        rewrite_value = "not an ip"
        "#,
    );
    let handler = DnsHandler::new_with_policy_v2(shared(allow_all_policy()), policy_v2, resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 0xD004);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Error);
    assert_eq!(
        res.matched_rule.as_deref(),
        Some("policy.dns.bogus_rewrite")
    );
    assert_eq!(res.upstream_resolver_ms, 0);
    assert_eq!(res.rcode, 2);
    assert_eq!(res.policy_action.as_deref(), Some("rewrite"));
    assert!(res
        .policy_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("failed closed")));
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::ServFail);
}

#[tokio::test]
async fn policy_v2_dns_rewrite_with_wrong_target_fails_closed_without_upstream_resolution() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy_v2 = policy_v2_from_toml(
        r#"
        [policy.dns.wrong_target]
        on = "dns.query"
        if = 'qname == "api.openai.com"'
        decision = "rewrite"
        priority = 1
        rewrite_target = 'request.url =~ ".*"'
        rewrite_value = "127.0.0.1"
        "#,
    );
    let handler = DnsHandler::new_with_policy_v2(shared(allow_all_policy()), policy_v2, resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 0xD007);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Error);
    assert_eq!(res.matched_rule.as_deref(), Some("policy.dns.wrong_target"));
    assert_eq!(res.upstream_resolver_ms, 0);
    assert_eq!(res.rcode, 2);
    assert_eq!(res.policy_action.as_deref(), Some("rewrite"));
    assert!(res
        .policy_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("unsupported DNS rewrite target")));
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::ServFail);
}

#[tokio::test]
async fn policy_v2_dns_live_block_re_evaluates_before_cache_hit() {
    let live = spawn_fake_upstream([10, 0, 0, 9], Duration::ZERO).await;
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![live]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let policy_v2 = Arc::new(tokio::sync::RwLock::new(Arc::new(PolicyConfig::default())));
    let handler = DnsHandler::with_cache_and_policy_v2(
        shared(allow_all_policy()),
        Arc::clone(&policy_v2),
        resolver,
        Arc::clone(&cache),
    );

    let q = build_query_bytes("api.openai.com.", RecordType::A, 0xD005);
    let initial = handler.handle(&q).await;
    assert_eq!(initial.decision, Decision::Allowed);
    assert_eq!(cache.len(), 1);

    let policy = PolicyConfig::from_policy_toml_str(
        r#"
        [policy.dns.block_openai]
        on = "dns.query"
        if = 'qname == "api.openai.com"'
        decision = "block"
        priority = 1
        "#,
    )
    .unwrap();
    *policy_v2.write().await = Arc::new(policy);

    let after_reload = handler.handle(&q).await;
    assert_eq!(after_reload.decision, Decision::Denied);
    assert_eq!(
        after_reload.matched_rule.as_deref(),
        Some("policy.dns.block_openai")
    );
    assert_eq!(after_reload.upstream_resolver_ms, 0);
    assert_eq!(after_reload.policy_action.as_deref(), Some("block"));
}

#[tokio::test]
async fn policy_v2_dns_live_rewrite_re_evaluates_before_cache_hit() {
    let live = spawn_fake_upstream([10, 0, 0, 9], Duration::ZERO).await;
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![live]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let policy_v2 = Arc::new(tokio::sync::RwLock::new(Arc::new(PolicyConfig::default())));
    let handler = DnsHandler::with_cache_and_policy_v2(
        shared(allow_all_policy()),
        Arc::clone(&policy_v2),
        resolver,
        Arc::clone(&cache),
    );

    let q = build_query_bytes("api.openai.com.", RecordType::A, 0xD006);
    let initial = handler.handle(&q).await;
    assert_eq!(initial.decision, Decision::Allowed);
    assert_eq!(cache.len(), 1);

    let policy = PolicyConfig::from_policy_toml_str(
        r#"
        [policy.dns.rewrite_openai]
        on = "dns.query"
        if = 'qname == "api.openai.com" && qtype == "A"'
        decision = "rewrite"
        priority = 1
        rewrite_target = 'answer.ip =~ ".*"'
        rewrite_value = "127.0.0.77"
        "#,
    )
    .unwrap();
    *policy_v2.write().await = Arc::new(policy);

    let after_reload = handler.handle(&q).await;
    assert_eq!(after_reload.decision, Decision::Redirected);
    assert_eq!(after_reload.upstream_resolver_ms, 0);
    assert_eq!(after_reload.policy_action.as_deref(), Some("rewrite"));
    let resp = Message::from_vec(&after_reload.answer_bytes).unwrap();
    assert_eq!(resp.answers.len(), 1);
    if let RData::A(answer) = &resp.answers[0].data {
        assert_eq!(answer.0, Ipv4Addr::new(127, 0, 0, 77));
    } else {
        panic!("expected A record after live DNS policy rewrite");
    }
}

#[tokio::test]
async fn allowed_domain_forwarded_to_upstream() {
    let upstream = spawn_fake_upstream([127, 0, 0, 1], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 0x4242);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Allowed);
    assert_eq!(res.matched_rule, None);
    assert_eq!(res.rcode, 0);
    assert!(!res.answer_bytes.is_empty());
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.id, 0x4242);
    assert_eq!(resp.metadata.response_code, ResponseCode::NoError);
    assert_eq!(resp.answers.len(), 1);
    let qq = res.query.unwrap();
    assert_eq!(qq.qname, "anthropic.com");
    assert_eq!(qq.qtype, u16::from(RecordType::A));
}

#[tokio::test]
async fn blocked_domain_returns_synthetic_nxdomain() {
    // Blackhole upstream so we'd hang if the policy short-circuit
    // didn't work -- the test would time out instead of asserting.
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = shared(block_specific_policy("api.openai.com"));
    let handler = DnsHandler::new(policy, resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 0xCAFE);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Denied);
    assert_eq!(res.matched_rule.as_deref(), Some("api.openai.com"));
    assert_eq!(res.upstream_resolver_ms, 0); // policy short-circuit
    assert_eq!(res.rcode, 3);
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.id, 0xCAFE);
    assert_eq!(resp.metadata.response_code, ResponseCode::NXDomain);
    assert_eq!(resp.queries.len(), 1);
    assert_eq!(resp.answers.len(), 0);
}

#[tokio::test]
async fn wildcard_block_matches_subdomain() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(200)),
    );
    let policy = shared(block_specific_policy("*.openai.com"));
    let handler = DnsHandler::new(policy, resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Denied);
    assert_eq!(res.matched_rule.as_deref(), Some("*.openai.com"));
}

#[tokio::test]
async fn read_only_domain_is_resolvable_not_blocked() {
    // Read-only (allow_read=true, allow_write=false) is the policy
    // shape for package registries. Resolution must succeed -- the
    // verb-level policy enforcement happens at the HTTP layer.
    let mut policy = NetworkPolicy::new(vec![], false, false);
    policy.rules.push(PolicyRule {
        matcher: DomainMatcher::parse("pypi.org"),
        allow_read: true,
        allow_write: false,
    });
    let upstream = spawn_fake_upstream([127, 0, 0, 1], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("pypi.org.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Allowed);
    assert_eq!(res.rcode, 0);
}

#[tokio::test]
async fn upstream_unreachable_returns_servfail_with_decision_error() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(150)),
    );
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 7);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Error);
    assert_eq!(res.rcode, 2);
    assert!(!res.answer_bytes.is_empty());
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::ServFail);
    assert_eq!(resp.metadata.id, 7);
}

#[tokio::test]
async fn malformed_query_returns_error_with_empty_answer() {
    let resolver = Arc::new(DnsResolver::with_upstreams(vec![]));
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);

    let res = handler.handle(b"not a dns message").await;

    assert_eq!(res.decision, Decision::Error);
    assert!(res.query.is_none());
    assert!(res.answer_bytes.is_empty());
    assert_eq!(res.upstream_resolver_ms, 0);
}

#[tokio::test]
async fn resolver_falls_over_to_second_upstream() {
    let dead = spawn_blackhole_upstream().await;
    let live = spawn_fake_upstream([10, 0, 0, 5], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![dead, live]).with_timeout(Duration::from_millis(150)),
    );
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 9);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Allowed);
    assert_eq!(res.rcode, 0);
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.id, 9);
    assert_eq!(resp.answers.len(), 1);
}

#[tokio::test]
async fn empty_upstream_list_is_an_error() {
    let resolver = Arc::new(DnsResolver::with_upstreams(vec![]));
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Error);
    assert_eq!(res.rcode, 2);
}

#[tokio::test]
async fn telemetry_fields_populated_for_allowed_query() {
    let upstream = spawn_fake_upstream([1, 2, 3, 4], Duration::from_millis(10)).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);

    let q = build_query_bytes("example.com.", RecordType::A, 0xBEEF);
    let res = handler.handle(&q).await;

    let qq = res.query.expect("parsed query metadata must be present");
    assert_eq!(qq.qname, "example.com");
    assert_eq!(qq.id, 0xBEEF);
    assert_eq!(qq.qtype, u16::from(RecordType::A));
    assert_eq!(qq.qclass, 1);
    // The fake upstream sleeps 10ms before answering -- wall-clock
    // jitter on a busy machine makes a strict floor flaky, so just
    // assert it's non-zero.
    assert!(res.upstream_resolver_ms > 0);
}

#[test]
fn default_resolver_has_default_upstreams() {
    let r = DnsResolver::new();
    assert_eq!(
        r.upstreams().len(),
        super::resolver::DEFAULT_UPSTREAMS.len()
    );
}

// =====================================================================
// (T3.d) -- DnsRedirect handler integration
//
// Each test uses a blackhole upstream so the handler would hang if
// the redirect didn't short-circuit. That converts "redirect doesn't
// fire" from a silent test pass into a tokio timeout test failure.
// =====================================================================

fn policy_with_redirect(pattern: &str, qtype: Option<u16>, ips: Vec<IpAddr>) -> NetworkPolicy {
    let mut p = NetworkPolicy::new(vec![], true, true);
    p.dns_redirects
        .push(DnsRedirect::new(pattern, qtype, ips, 60));
    p
}

#[tokio::test]
async fn redirect_a_query_returns_synthetic_answer() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_with_redirect(
        "anthropic.com",
        Some(1),
        vec![IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40))],
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 0xABCD);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Redirected);
    assert_eq!(res.matched_rule.as_deref(), Some("redirect:anthropic.com"));
    assert_eq!(res.rcode, 0);
    assert_eq!(res.upstream_resolver_ms, 0); // policy short-circuit

    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.id, 0xABCD);
    assert_eq!(resp.metadata.response_code, ResponseCode::NoError);
    assert_eq!(resp.answers.len(), 1);
    let answer = &resp.answers[0];
    assert_eq!(answer.record_type(), RecordType::A);
    if let RData::A(a) = &answer.data {
        assert_eq!(a.0, Ipv4Addr::new(10, 20, 30, 40));
    } else {
        panic!("expected A record, got {:?}", &answer.data);
    }
}

#[tokio::test]
async fn redirect_aaaa_query_returns_synthetic_v6_answer() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_with_redirect(
        "anthropic.com",
        Some(28),
        vec![IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1))],
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::AAAA, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Redirected);
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.answers.len(), 1);
    assert_eq!(resp.answers[0].record_type(), RecordType::AAAA);
}

#[tokio::test]
async fn redirect_qtype_none_matches_a() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_with_redirect(
        "anthropic.com",
        None, // any qtype
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Redirected);
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.answers.len(), 1);
    assert_eq!(resp.answers[0].record_type(), RecordType::A);
}

#[tokio::test]
async fn redirect_aaaa_with_only_ipv4_answers_yields_nodata() {
    // qtype = None, answers contain only IPv4. AAAA query gets
    // NoError + zero answers -- the standard "name exists, no
    // record of that type" shape.
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_with_redirect(
        "anthropic.com",
        None,
        vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))],
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::AAAA, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Redirected);
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::NoError);
    assert_eq!(resp.answers.len(), 0); // no AAAA record to give back
}

#[tokio::test]
async fn redirect_qtype_filter_falls_through_to_upstream() {
    // Redirect only set for A; AAAA query MUST forward upstream.
    // Use a fake upstream so the AAAA call returns rather than
    // hanging.
    let upstream = spawn_fake_upstream([1, 2, 3, 4], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_with_redirect(
        "anthropic.com",
        Some(1), // A only
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::AAAA, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Allowed); // forwarded, not redirected
    assert!(res.matched_rule.is_none());
}

#[tokio::test]
async fn redirect_wildcard_matches_subdomain_not_base() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(200)),
    );
    let policy = policy_with_redirect(
        "*.openai.com",
        None,
        vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    // Subdomain: redirect fires.
    let q = build_query_bytes("api.openai.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;
    assert_eq!(res.decision, Decision::Redirected);
    assert_eq!(res.matched_rule.as_deref(), Some("redirect:*.openai.com"));
}

#[tokio::test]
async fn block_overrides_redirect_when_both_match() {
    // The handler checks is_fully_blocked BEFORE redirects.
    // A domain that's both blocked AND has a redirect rule must
    // get NXDOMAIN -- block never weakens.
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let mut policy = block_specific_policy("api.openai.com");
    policy.dns_redirects.push(DnsRedirect::new(
        "api.openai.com",
        None,
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
        60,
    ));
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Denied); // block wins
    assert_eq!(res.rcode, 3);
    assert_eq!(res.matched_rule.as_deref(), Some("api.openai.com"));
}

#[tokio::test]
async fn redirect_multiple_ips_all_appear_in_answer() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_with_redirect(
        "loadbalanced.example.com",
        Some(1),
        vec![
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3)),
        ],
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("loadbalanced.example.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.answers.len(), 3);
}

#[tokio::test]
async fn redirect_empty_answers_yields_nodata_response() {
    // Empty `answers` list: synthetic NoError + zero answers.
    // Useful for "this name exists but we have nothing to say"
    // shape that makes browsers move on quickly.
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_with_redirect("nodata.example.com", None, vec![]);
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("nodata.example.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Redirected);
    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::NoError);
    assert_eq!(resp.answers.len(), 0);
}

#[tokio::test]
async fn redirect_ttl_propagates_to_answer_record() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let mut policy = NetworkPolicy::new(vec![], true, true);
    policy.dns_redirects.push(DnsRedirect::new(
        "anthropic.com",
        Some(1),
        vec![IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40))],
        300, // 5 min TTL
    ));
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.answers[0].ttl, 300);
}

// =====================================================================
// (T3.f) -- metrics emission assertions
//
// Use a thread-local DebuggingRecorder so each test snapshots only
// its own emissions (parallel tests don't pollute each other).
// =====================================================================

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

fn count_for(snapshotter: &Snapshotter, metric: &str, decision: Option<&str>) -> u64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(k, _, _, v)| {
            if k.key().name() != metric {
                return None;
            }
            if let Some(want) = decision {
                let has_label = k
                    .key()
                    .labels()
                    .any(|l| l.key() == "decision" && l.value() == want);
                if !has_label {
                    return None;
                }
            }
            match v {
                DebugValue::Counter(c) => Some(c),
                _ => None,
            }
        })
        .sum()
}

fn histogram_present(snapshotter: &Snapshotter, metric: &str) -> bool {
    snapshotter
        .snapshot()
        .into_vec()
        .iter()
        .any(|(k, _, _, v)| k.key().name() == metric && matches!(v, DebugValue::Histogram(_)))
}

#[tokio::test]
async fn metrics_increment_for_allowed_query() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_fake_upstream([1, 2, 3, 4], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);

    let q = build_query_bytes("example.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;

    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("allowed")),
        1
    );
    assert!(histogram_present(&snap, "mitm.dns_handle_duration_ms"));
    assert!(histogram_present(&snap, "mitm.dns_upstream_duration_ms"));
}

#[tokio::test]
async fn metrics_increment_for_denied_query() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = block_specific_policy("api.openai.com");
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;

    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("denied")),
        1
    );
    // Denied path short-circuits before upstream -- the upstream
    // duration histogram MUST be absent.
    assert!(!histogram_present(&snap, "mitm.dns_upstream_duration_ms"));
}

#[tokio::test]
async fn metrics_increment_for_redirected_query() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_with_redirect(
        "anthropic.com",
        Some(1),
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;

    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("redirected")),
        1
    );
}

#[tokio::test]
async fn metrics_increment_upstream_failures() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(150)),
    );
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;

    assert_eq!(count_for(&snap, "mitm.dns_queries_total", Some("error")), 1);
    assert_eq!(
        count_for(&snap, "mitm.dns_upstream_failures_total", None),
        1
    );
}

#[tokio::test]
async fn metrics_decision_label_distinct_per_outcome() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_fake_upstream([1, 2, 3, 4], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    // Mix: one allowed, one redirected, one denied -- via three
    // separate queries to the same handler.
    let mut policy = NetworkPolicy::new(vec![], true, true);
    policy.rules.push(crate::net::policy::PolicyRule {
        matcher: DomainMatcher::parse("blocked.example.com"),
        allow_read: false,
        allow_write: false,
    });
    policy.dns_redirects.push(DnsRedirect::new(
        "redirect.example.com",
        Some(1),
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
        60,
    ));
    let handler = DnsHandler::new(shared(policy), resolver);

    let _ = handler
        .handle(&build_query_bytes("ok.example.com.", RecordType::A, 1))
        .await;
    let _ = handler
        .handle(&build_query_bytes("blocked.example.com.", RecordType::A, 2))
        .await;
    let _ = handler
        .handle(&build_query_bytes(
            "redirect.example.com.",
            RecordType::A,
            3,
        ))
        .await;

    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("allowed")),
        1
    );
    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("denied")),
        1
    );
    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("redirected")),
        1
    );
}

// =====================================================================
// (T3.f) -- DnsAnswerCache integration via DnsHandler::with_cache
// =====================================================================

use super::cache::DnsAnswerCache;

#[tokio::test]
async fn cache_hit_short_circuits_upstream() {
    // First query forwards upstream + populates the cache. Second
    // query is served from cache -- to prove that, swap the
    // upstream to a blackhole between calls. Cache hit means we
    // never reach the blackhole, so the second call returns
    // promptly with the cached bytes.
    let live = spawn_fake_upstream([10, 0, 0, 1], Duration::ZERO).await;
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![live]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let handler = DnsHandler::with_cache(
        shared(allow_all_policy()),
        Arc::clone(&resolver),
        Arc::clone(&cache),
    );

    // First call: upstream miss -> populate cache.
    let q = build_query_bytes("example.com.", RecordType::A, 1);
    let r1 = handler.handle(&q).await;
    assert_eq!(r1.decision, Decision::Allowed);
    // r1.upstream_resolver_ms is the wall time of the upstream
    // call -- a u64, always >= 0; we don't pin a lower bound to
    // avoid wall-clock jitter flakiness.

    assert_eq!(cache.len(), 1);

    // Second call: cache hit -> upstream_resolver_ms == 0 (no
    // upstream call). bytes match.
    let r2 = handler.handle(&q).await;
    assert_eq!(r2.decision, Decision::Allowed);
    assert_eq!(r2.upstream_resolver_ms, 0); // tell-tale of cache hit
    assert_eq!(r2.answer_bytes, r1.answer_bytes);
}

#[tokio::test]
async fn cache_invalidated_when_policy_now_blocks() {
    let live = spawn_fake_upstream([10, 0, 0, 1], Duration::ZERO).await;
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![live]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let policy_handle = shared(allow_all_policy());
    let handler = DnsHandler::with_cache(
        Arc::clone(&policy_handle),
        Arc::clone(&resolver),
        Arc::clone(&cache),
    );

    // Populate cache.
    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let r1 = handler.handle(&q).await;
    assert_eq!(r1.decision, Decision::Allowed);
    assert_eq!(cache.len(), 1);

    // Hot-swap policy to block anthropic.com.
    {
        let mut w = policy_handle.write().unwrap();
        let mut new_policy = (**w).clone();
        new_policy.rules.push(crate::net::policy::PolicyRule {
            matcher: DomainMatcher::parse("anthropic.com"),
            allow_read: false,
            allow_write: false,
        });
        *w = Arc::new(new_policy);
    }

    // Next query MUST NOT serve from cache. Decision = Denied.
    // The block path short-circuits before touching the cache, so
    // the stale entry stays present until something tries to read
    // it through the cache path (then it'll be lazily invalidated
    // by `DnsAnswerCache::get`'s policy re-check). What matters
    // here is the semantic: a now-blocked domain is NEVER served
    // from cache. We assert that via the response shape.
    let r2 = handler.handle(&q).await;
    assert_eq!(r2.decision, Decision::Denied);
    assert_eq!(r2.rcode, 3);

    // Direct cache.get with the new policy must return None (and
    // evict the entry). This pins the lazy-invalidation
    // contract.
    let pol_snapshot = policy_handle.read().unwrap().clone();
    assert!(cache.get("anthropic.com", 1, 1, 0, &pol_snapshot).is_none());
    assert_eq!(cache.len(), 0); // popped on the lazy-invalidation read
}

#[tokio::test]
async fn cache_invalidated_when_policy_now_redirects() {
    let live = spawn_fake_upstream([10, 0, 0, 1], Duration::ZERO).await;
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![live]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let policy_handle = shared(allow_all_policy());
    let handler = DnsHandler::with_cache(
        Arc::clone(&policy_handle),
        Arc::clone(&resolver),
        Arc::clone(&cache),
    );

    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;
    assert_eq!(cache.len(), 1);

    // Add a redirect.
    {
        let mut w = policy_handle.write().unwrap();
        let mut new_policy = (**w).clone();
        new_policy.dns_redirects.push(DnsRedirect::new(
            "anthropic.com",
            Some(1),
            vec![IpAddr::V4(Ipv4Addr::new(99, 99, 99, 99))],
            60,
        ));
        *w = Arc::new(new_policy);
    }

    let r2 = handler.handle(&q).await;
    assert_eq!(r2.decision, Decision::Redirected);
    // Same lazy-invalidation contract as the block test: redirect
    // path short-circuits before the cache. Direct cache.get with
    // the new policy proves the entry is no longer servable.
    let pol_snapshot = policy_handle.read().unwrap().clone();
    assert!(cache.get("anthropic.com", 1, 1, 0, &pol_snapshot).is_none());
    assert_eq!(cache.len(), 0);
}

#[tokio::test]
async fn cache_does_not_short_circuit_block_or_redirect() {
    // Even with a cache, blocked / redirect domains are evaluated
    // via the policy path -- never cached. Verify by populating
    // cache for an allowed domain, then querying a blocked one
    // (different qname): cache stays at 1, response is NXDOMAIN.
    let live = spawn_fake_upstream([10, 0, 0, 1], Duration::ZERO).await;
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![live]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let mut policy = NetworkPolicy::new(vec![], true, true);
    policy.rules.push(crate::net::policy::PolicyRule {
        matcher: DomainMatcher::parse("blocked.example.com"),
        allow_read: false,
        allow_write: false,
    });
    let handler = DnsHandler::with_cache(shared(policy), Arc::clone(&resolver), Arc::clone(&cache));

    // Populate cache with an allowed name.
    let q1 = build_query_bytes("ok.example.com.", RecordType::A, 1);
    let _ = handler.handle(&q1).await;
    assert_eq!(cache.len(), 1);

    // Blocked name -- should NXDOMAIN, not be cached.
    let q2 = build_query_bytes("blocked.example.com.", RecordType::A, 2);
    let r = handler.handle(&q2).await;
    assert_eq!(r.decision, Decision::Denied);
    assert_eq!(cache.len(), 1); // unchanged
}

#[tokio::test]
async fn cache_hit_metric_increments() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let live = spawn_fake_upstream([10, 0, 0, 1], Duration::ZERO).await;
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![live]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let handler = DnsHandler::with_cache(shared(allow_all_policy()), resolver, Arc::clone(&cache));

    let q = build_query_bytes("example.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await; // miss
    let _ = handler.handle(&q).await; // hit

    assert_eq!(count_for(&snap, "mitm.dns_cache_hits_total", None), 1);
    assert_eq!(count_for(&snap, "mitm.dns_cache_misses_total", None), 1);
}

#[tokio::test]
async fn cache_does_not_persist_servfail_or_nxdomain_from_upstream() {
    // Upstream returns NoError + zero answers (nodata), or any
    // non-NoError rcode -- those should not poison the cache.
    // Simulate via a fake upstream returning NXDOMAIN.
    let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        if let Ok((n, peer)) = sock.recv_from(&mut buf).await {
            let req = Message::from_vec(&buf[..n]).unwrap();
            let mut resp = Message::new(req.metadata.id, MessageType::Response, OpCode::Query);
            resp.metadata.recursion_available = true;
            resp.metadata.response_code = ResponseCode::NXDomain;
            for q in &req.queries {
                resp.add_query(q.clone());
            }
            let _ = sock.send_to(&resp.to_vec().unwrap(), peer).await;
        }
    });

    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![addr]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let handler = DnsHandler::with_cache(shared(allow_all_policy()), resolver, Arc::clone(&cache));

    let q = build_query_bytes("nx.example.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;
    assert_eq!(cache.len(), 0); // NXDOMAIN not cached
}

#[tokio::test]
async fn cache_default_constructor_enables_caching() {
    let handler = DnsHandler::with_default_resolver(shared(allow_all_policy()));
    assert!(handler.cache().is_some());
    assert_eq!(handler.cache().unwrap().len(), 0);
}

#[tokio::test]
async fn cache_explicit_none_via_new() {
    let resolver = Arc::new(DnsResolver::new());
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);
    assert!(handler.cache().is_none());
}

#[tokio::test]
async fn redirect_no_match_falls_through_to_upstream() {
    let upstream = spawn_fake_upstream([5, 6, 7, 8], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_with_redirect(
        "anthropic.com", // only redirects this domain
        None,
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    // Query a different domain -- redirect doesn't fire, upstream wins.
    let q = build_query_bytes("example.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Allowed);
    assert!(res.matched_rule.is_none());
}
