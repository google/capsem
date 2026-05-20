use super::metrics_behavior::count_for;
use super::*;
use metrics_util::debugging::DebuggingRecorder;

// =====================================================================
// (T3.f) -- DnsAnswerCache integration via DnsHandler::with_cache
// =====================================================================

use crate::net::dns::cache::DnsAnswerCache;

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
async fn cache_not_served_when_policy_now_blocks() {
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

    let new_policy = PolicyConfig::from_policy_toml_str(
        r#"
        [policy.dns.block_anthropic]
        on = "dns.query"
        if = 'qname == "anthropic.com"'
        decision = "block"
        priority = 1
        "#,
    )
    .unwrap();
    *policy_handle.write().await = Arc::new(new_policy);

    // Next query MUST NOT serve from cache. Decision = Denied.
    // The block path short-circuits before touching the cache, so
    // stale cache bytes may remain internally but are not servable
    // through the handler.
    let r2 = handler.handle(&q).await;
    assert_eq!(r2.decision, Decision::Denied);
    assert_eq!(r2.rcode, 3);
    assert_eq!(cache.len(), 1);
}

#[tokio::test]
async fn cache_not_served_when_policy_now_redirects() {
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

    let new_policy = policy_with_redirect(
        "anthropic.com",
        Some(1),
        vec![IpAddr::V4(Ipv4Addr::new(99, 99, 99, 99))],
    );
    *policy_handle.write().await = Arc::new(new_policy);

    let r2 = handler.handle(&q).await;
    assert_eq!(r2.decision, Decision::Redirected);
    let resp = Message::from_vec(&r2.answer_bytes).unwrap();
    if let RData::A(answer) = &resp.answers[0].data {
        assert_eq!(answer.0, Ipv4Addr::new(99, 99, 99, 99));
    } else {
        panic!("expected A record after live DNS policy rewrite");
    }
    assert_eq!(cache.len(), 1);
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
    let policy = block_specific_policy("blocked.example.com");
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
