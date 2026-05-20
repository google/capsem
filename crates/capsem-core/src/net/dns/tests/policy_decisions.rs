use super::*;
use crate::net::dns::DnsAnswerCache;

#[tokio::test]
async fn policy_dns_block_returns_nxdomain_without_upstream_and_records_policy_fields() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_from_toml(
        r#"
        [policy.dns.block_openai]
        on = "dns.query"
        if = 'qname == "api.openai.com" && qtype == "A"'
        decision = "block"
        priority = 10
        reason = "DNS to OpenAI API is blocked"
        "#,
    );
    let handler = DnsHandler::new(policy, resolver);

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
async fn policy_dns_allow_forwards_upstream_and_records_policy_fields() {
    let upstream = spawn_fake_upstream([10, 11, 12, 13], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_from_toml(
        r#"
        [policy.dns.allow_openai]
        on = "dns.query"
        if = 'qname == "api.openai.com" && qtype == "A"'
        decision = "allow"
        priority = 1
        reason = "DNS to OpenAI API is allowed"
        "#,
    );
    let handler = DnsHandler::new(policy, resolver);

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
async fn policy_dns_ask_fails_closed_without_upstream_resolution() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_from_toml(
        r#"
        [policy.dns.ask_openai]
        on = "dns.query"
        if = 'qname == "api.openai.com"'
        decision = "ask"
        priority = 5
        reason = "DNS query needs approval"
        "#,
    );
    let handler = DnsHandler::new(policy, resolver);

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
async fn policy_dns_rewrite_synthesizes_answer_without_upstream_resolution() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_from_toml(
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
    let handler = DnsHandler::new(policy, resolver);

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
async fn policy_dns_rewrite_with_invalid_answer_fails_closed_without_upstream_resolution() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_from_toml(
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
    let handler = DnsHandler::new(policy, resolver);

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
async fn policy_dns_rewrite_with_wrong_target_fails_closed_without_upstream_resolution() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_from_toml(
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
    let handler = DnsHandler::new(policy, resolver);

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
async fn policy_dns_live_block_re_evaluates_before_cache_hit() {
    let live = spawn_fake_upstream([10, 0, 0, 9], Duration::ZERO).await;
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![live]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let policy_handle = Arc::new(tokio::sync::RwLock::new(Arc::new(PolicyConfig::default())));
    let handler = DnsHandler::with_cache(Arc::clone(&policy_handle), resolver, Arc::clone(&cache));

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
    *policy_handle.write().await = Arc::new(policy);

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
async fn policy_dns_live_rewrite_re_evaluates_before_cache_hit() {
    let live = spawn_fake_upstream([10, 0, 0, 9], Duration::ZERO).await;
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![live]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let policy_handle = Arc::new(tokio::sync::RwLock::new(Arc::new(PolicyConfig::default())));
    let handler = DnsHandler::with_cache(Arc::clone(&policy_handle), resolver, Arc::clone(&cache));

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
    *policy_handle.write().await = Arc::new(policy);

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
    assert_eq!(res.matched_rule.as_deref(), Some("policy.dns.block_domain"));
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
    assert_eq!(res.matched_rule.as_deref(), Some("policy.dns.block_domain"));
}

#[tokio::test]
async fn dns_allow_rule_is_resolvable_not_blocked() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
        [policy.dns.allow_pypi]
        on = "dns.query"
        if = 'qname == "pypi.org"'
        decision = "allow"
        priority = 1
        "#,
    )
    .unwrap();
    let upstream = spawn_fake_upstream([127, 0, 0, 1], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("pypi.org.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Allowed);
    assert_eq!(res.rcode, 0);
    assert_eq!(res.policy_action.as_deref(), Some("allow"));
    assert_eq!(res.policy_rule.as_deref(), Some("policy.dns.allow_pypi"));
}
