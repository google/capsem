use super::*;

// =====================================================================
// (T3.d) -- Policy DNS rewrite handler integration
//
// Each test uses a blackhole upstream so the handler would hang if
// the redirect didn't short-circuit. That converts "redirect doesn't
// fire" from a silent test pass into a tokio timeout test failure.
// =====================================================================

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
    assert_eq!(
        res.matched_rule.as_deref(),
        Some("policy.dns.rewrite_domain")
    );
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
    assert_eq!(
        res.matched_rule.as_deref(),
        Some("policy.dns.rewrite_domain")
    );
}

#[tokio::test]
async fn block_overrides_redirect_when_both_match() {
    // Higher-priority block rules must win over lower-priority
    // rewrite rules for the same DNS query.
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
        [policy.dns.block_api]
        on = "dns.query"
        if = 'qname == "api.openai.com"'
        decision = "block"
        priority = 1

        [policy.dns.rewrite_api]
        on = "dns.query"
        if = 'qname == "api.openai.com"'
        decision = "rewrite"
        priority = 10
        rewrite_target = 'answer.ip =~ ".*"'
        rewrite_value = "127.0.0.1"
        "#,
    )
    .unwrap();
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    assert_eq!(res.decision, Decision::Denied); // block wins
    assert_eq!(res.rcode, 3);
    assert_eq!(res.matched_rule.as_deref(), Some("policy.dns.block_api"));
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
async fn redirect_uses_short_answer_ttl() {
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

    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let res = handler.handle(&q).await;

    let resp = Message::from_vec(&res.answer_bytes).unwrap();
    assert_eq!(resp.answers[0].ttl, 60);
}
