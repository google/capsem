use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RecordType};

fn build_query_bytes(name: &str, qtype: RecordType, id: u16) -> Vec<u8> {
    let mut msg = Message::new(id, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    let name = Name::from_ascii(name).unwrap();
    msg.add_query(Query::query(name, qtype));
    msg.to_vec().unwrap()
}

fn shared_policy() -> SharedPolicy {
    Arc::new(std::sync::RwLock::new(Arc::new(NetworkMechanics::new())))
}

fn security_rules(toml: &str) -> SharedSecurityRules {
    let profile = crate::net::policy_config::SecurityRuleProfile::parse_toml(toml).unwrap();
    let rules =
        SecurityRuleSet::compile_profile(&profile, crate::net::policy_config::SecurityRuleSource::User).unwrap();
    Arc::new(std::sync::RwLock::new(Arc::new(rules)))
}

fn plugin_policy() -> SharedPluginPolicy {
    Arc::new(std::sync::RwLock::new(Arc::new(BTreeMap::new())))
}

#[tokio::test]
async fn only_a_single_standard_question_can_reach_upstream() {
    let (addr, seen) = counting_upstream(hickory_proto::op::ResponseCode::NoError, std::time::Duration::ZERO).await;
    let handler = DnsHandler::new(
        shared_policy(),
        security_rules(""),
        plugin_policy(),
        Arc::new(DnsResolver::with_upstreams(vec![addr])),
    );
    let ordinary = build_query_bytes("allowed.example.", RecordType::A, 123);
    let mut multiple = Message::from_vec(&ordinary).unwrap();
    multiple.add_query(Query::query(
        Name::from_ascii("blocked.example.").unwrap(),
        RecordType::A,
    ));
    let mut response = ordinary.clone();
    response[2] |= 0x80;
    let mut update = ordinary;
    update[2] |= 5 << 3;
    for query in [multiple.to_vec().unwrap(), response, update] {
        let result = handler.handle(&query).await;
        assert_eq!(result.decision, Decision::Error);
        assert_eq!(
            seen.load(Ordering::SeqCst),
            0,
            "unchecked questions or operations must never be forwarded"
        );
    }
}

#[tokio::test]
async fn dns_handler_blocks_query_through_security_event_rules() {
    let handler = DnsHandler::new(
        shared_policy(),
        security_rules(
            r#"
            [profiles.rules.block_dns_example]
            name = "block_dns_example"
            action = "block"
            reason = "dns test block"
            match = 'dns.qname == "blocked.example.com"'
            "#,
        ),
        plugin_policy(),
        Arc::new(DnsResolver::new()),
    );

    let result = handler
        .handle(&build_query_bytes("blocked.example.com.", RecordType::A, 0xCAFE))
        .await;

    assert_eq!(result.decision, Decision::Denied);
    assert_eq!(result.rcode, 3);
    assert_eq!(result.upstream_resolver_ms, 0);
    assert_eq!(result.matched_rule.as_deref(), Some("profiles.rules.block_dns_example"));
    assert_eq!(result.policy_mode.as_deref(), Some("security_event"));
    assert_eq!(result.policy_action.as_deref(), Some("block"));
    assert_eq!(result.policy_rule.as_deref(), Some("profiles.rules.block_dns_example"));
    assert_eq!(result.policy_reason.as_deref(), Some("dns test block"));
}

#[tokio::test]
async fn dns_handler_returns_local_nxdomain_for_capsem_bogus_without_upstream() {
    let handler = DnsHandler::new(
        shared_policy(),
        security_rules(""),
        plugin_policy(),
        Arc::new(DnsResolver::with_upstreams(Vec::new())),
    );

    let result = handler
        .handle(&build_query_bytes("load-test.capsem-bogus.", RecordType::A, 0xBEEF))
        .await;

    assert_eq!(result.decision, Decision::Denied);
    assert_eq!(result.rcode, 3);
    assert_eq!(result.upstream_resolver_ms, 0);
    assert_eq!(result.matched_rule.as_deref(), Some(CAPSEM_LOCAL_NXDOMAIN_RULE));
    assert_eq!(result.policy_action.as_deref(), Some("allow"));
    assert_eq!(result.policy_mode.as_deref(), Some("security_event"));
    assert!(!result.answer_bytes.is_empty());
}

/// A fake upstream that answers every query with `rcode` (NoError with one
/// A record, or NXDOMAIN) after `delay`, counting the datagrams it saw.
async fn counting_upstream(
    rcode: hickory_proto::op::ResponseCode,
    delay: std::time::Duration,
) -> (std::net::SocketAddr, Arc<AtomicUsize>) {
    use hickory_proto::rr::{rdata, RData, Record};
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        while let Ok((n, peer)) = socket.recv_from(&mut buf).await {
            counter.fetch_add(1, Ordering::SeqCst);
            let request = Message::from_vec(&buf[..n]).unwrap();
            let mut response = Message::new(request.metadata.id, MessageType::Response, OpCode::Query);
            response.metadata.recursion_available = true;
            response.metadata.response_code = rcode;
            response.add_queries(request.queries.iter().cloned());
            if rcode == hickory_proto::op::ResponseCode::NoError {
                let name = request.queries[0].name().clone();
                response.add_answer(Record::from_rdata(
                    name,
                    60,
                    RData::A(rdata::A(std::net::Ipv4Addr::new(192, 0, 2, 7))),
                ));
            } else if rcode == hickory_proto::op::ResponseCode::NXDomain {
                response.add_authority(Record::from_rdata(
                    Name::from_ascii("example.com.").unwrap(),
                    60,
                    RData::SOA(rdata::SOA::new(
                        Name::from_ascii("ns.example.com.").unwrap(),
                        Name::from_ascii("hostmaster.example.com.").unwrap(),
                        1,
                        3600,
                        600,
                        86400,
                        60,
                    )),
                ));
            }
            let bytes = response.to_vec().unwrap();
            let socket_send = socket.send_to(&bytes, peer);
            tokio::time::sleep(delay).await;
            socket_send.await.unwrap();
        }
    });
    (addr, seen)
}

fn handler_with(upstream: std::net::SocketAddr, rules: &str, cache: Option<Arc<DnsAnswerCache>>) -> DnsHandler {
    let resolver = Arc::new(DnsResolver::with_upstreams(vec![upstream]));
    match cache {
        Some(cache) => DnsHandler::with_cache(shared_policy(), security_rules(rules), plugin_policy(), resolver, cache),
        None => DnsHandler::new(shared_policy(), security_rules(rules), plugin_policy(), resolver),
    }
}

#[tokio::test]
async fn fifty_identical_queries_cost_one_upstream_lookup_and_each_gets_its_own_id() {
    let (upstream, seen) = counting_upstream(
        hickory_proto::op::ResponseCode::NoError,
        std::time::Duration::from_millis(50),
    )
    .await;
    let handler = Arc::new(handler_with(upstream, "", None));
    let mut tasks = Vec::new();
    for id in 1..=50u16 {
        let handler = Arc::clone(&handler);
        tasks.push(tokio::spawn(async move {
            let result = handler
                .handle(&build_query_bytes("shared.example.com.", RecordType::A, id))
                .await;
            assert_eq!(result.decision, Decision::Allowed);
            assert_eq!(result.rcode, 0);
            assert_eq!(&result.answer_bytes[0..2], &id.to_be_bytes(), "own transaction id");
            let message = Message::from_vec(&result.answer_bytes).unwrap();
            assert_eq!(message.queries[0].name().to_string(), "shared.example.com.");
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(
        seen.load(Ordering::SeqCst),
        1,
        "one upstream datagram for fifty concurrent queries"
    );
}

#[tokio::test]
async fn a_different_record_type_is_its_own_upstream_lookup() {
    let (upstream, seen) = counting_upstream(
        hickory_proto::op::ResponseCode::NoError,
        std::time::Duration::from_millis(30),
    )
    .await;
    let handler = Arc::new(handler_with(upstream, "", None));
    let a = {
        let h = Arc::clone(&handler);
        tokio::spawn(async move {
            h.handle(&build_query_bytes("types.example.com.", RecordType::A, 1))
                .await
        })
    };
    let aaaa = {
        let h = Arc::clone(&handler);
        tokio::spawn(async move {
            h.handle(&build_query_bytes("types.example.com.", RecordType::AAAA, 2))
                .await
        })
    };
    a.await.unwrap();
    aaaa.await.unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn different_dns_flags_do_not_share_an_upstream_answer() {
    let (upstream, seen) = counting_upstream(
        hickory_proto::op::ResponseCode::NoError,
        std::time::Duration::from_millis(20),
    )
    .await;
    let handler = handler_with(upstream, "", None);
    let first = build_query_bytes("flags.example.", RecordType::A, 1);
    let mut second = build_query_bytes("flags.example.", RecordType::A, 2);
    second[3] |= 0x10; // CD: the client requests different validation behavior.
    let (a, b) = tokio::join!(handler.handle(&first), handler.handle(&second));
    assert_eq!((a.rcode, b.rcode), (0, 0));
    assert_eq!(
        seen.load(Ordering::SeqCst),
        2,
        "different flags need distinct upstream reads"
    );
}

#[tokio::test]
async fn different_dns_flags_do_not_reuse_a_cached_answer() {
    let (upstream, seen) = counting_upstream(hickory_proto::op::ResponseCode::NoError, std::time::Duration::ZERO).await;
    let handler = handler_with(upstream, "", Some(Arc::new(DnsAnswerCache::default())));
    let first = build_query_bytes("flags.example.", RecordType::A, 1);
    let mut second = build_query_bytes("flags.example.", RecordType::A, 2);
    second[3] |= 0x10;
    assert_eq!(handler.handle(&first).await.rcode, 0);
    assert_eq!(handler.handle(&second).await.rcode, 0);
    assert_eq!(
        seen.load(Ordering::SeqCst),
        2,
        "a cached validated answer is not an answer to a CD query"
    );
}

#[tokio::test]
async fn a_blocked_query_is_never_joined_to_an_allowed_lookup() {
    let (upstream, seen) = counting_upstream(
        hickory_proto::op::ResponseCode::NoError,
        std::time::Duration::from_millis(80),
    )
    .await;
    // One handler allows the name, another blocks it; both share the
    // upstream. The blocked handler's query must never wait for, or
    // receive, the allowed lookup's answer.
    let allowing = Arc::new(handler_with(upstream, "", None));
    let blocking = handler_with(
        upstream,
        r#"
        [profiles.rules.block_shared]
        name = "block_shared"
        action = "block"
        match = 'dns.qname == "joined.example.com"'
        "#,
        None,
    );
    let leader = {
        let h = Arc::clone(&allowing);
        tokio::spawn(async move {
            h.handle(&build_query_bytes("joined.example.com.", RecordType::A, 1))
                .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let started = std::time::Instant::now();
    let denied = blocking
        .handle(&build_query_bytes("joined.example.com.", RecordType::A, 2))
        .await;
    assert_eq!(denied.decision, Decision::Denied);
    assert_eq!(denied.rcode, 3);
    assert!(
        started.elapsed() < std::time::Duration::from_millis(60),
        "a block does not wait on anyone"
    );
    let allowed = leader.await.unwrap();
    assert_eq!(allowed.decision, Decision::Allowed);
    assert_eq!(seen.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failed_leader_gives_every_follower_its_own_servfail_and_caches_nothing() {
    // Nothing listens here: every attempt times out.
    let sink = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let upstream = sink.local_addr().unwrap();
    let cache = Arc::new(DnsAnswerCache::default());
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![upstream]).with_timeout(std::time::Duration::from_millis(100)));
    let handler = Arc::new(DnsHandler::with_cache(
        shared_policy(),
        security_rules(""),
        plugin_policy(),
        resolver,
        Arc::clone(&cache),
    ));
    let mut tasks = Vec::new();
    for id in 1..=5u16 {
        let handler = Arc::clone(&handler);
        tasks.push(tokio::spawn(async move {
            handler
                .handle(&build_query_bytes("dead-upstream.example.", RecordType::A, id))
                .await
        }));
    }
    for (index, task) in tasks.into_iter().enumerate() {
        let result = task.await.unwrap();
        assert_eq!(result.rcode, 2, "SERVFAIL for query {index}");
        assert_eq!(&result.answer_bytes[0..2], &((index + 1) as u16).to_be_bytes());
    }
    assert!(cache.is_empty(), "a failure is never cached");
}

#[tokio::test]
async fn an_upstream_nxdomain_is_served_from_the_cache_on_the_next_query() {
    let (upstream, seen) =
        counting_upstream(hickory_proto::op::ResponseCode::NXDomain, std::time::Duration::ZERO).await;
    let cache = Arc::new(DnsAnswerCache::default());
    let handler = handler_with(upstream, "", Some(Arc::clone(&cache)));
    let first = handler
        .handle(&build_query_bytes("nope.example.com.", RecordType::A, 0x0A0A))
        .await;
    assert_eq!(first.rcode, 3);
    assert_eq!(
        first.decision,
        Decision::Allowed,
        "an upstream NXDOMAIN is not a policy denial"
    );
    let second = handler
        .handle(&build_query_bytes("nope.example.com.", RecordType::A, 0x0B0B))
        .await;
    assert_eq!(second.rcode, 3);
    assert_eq!(&second.answer_bytes[0..2], &[0x0B, 0x0B]);
    assert_eq!(second.upstream_resolver_ms, 0, "answered from the cache");
    assert_eq!(seen.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_denied_nxdomain_is_not_a_cached_negative_answer() {
    let (upstream, seen) = counting_upstream(hickory_proto::op::ResponseCode::NoError, std::time::Duration::ZERO).await;
    let cache = Arc::new(DnsAnswerCache::default());
    let handler = handler_with(
        upstream,
        r#"
        [profiles.rules.block_it]
        name = "block_it"
        action = "block"
        match = 'dns.qname == "denied.example.com"'
        "#,
        Some(Arc::clone(&cache)),
    );
    let result = handler
        .handle(&build_query_bytes("denied.example.com.", RecordType::A, 1))
        .await;
    assert_eq!(result.decision, Decision::Denied);
    assert_eq!(result.rcode, 3);
    assert!(cache.is_empty(), "a policy denial never enters the answer cache");
    assert_eq!(seen.load(Ordering::SeqCst), 0, "and never reaches the upstream");
}
