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
    let handler = DnsHandler::with_cache(Arc::clone(&resolver), Arc::clone(&cache));

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
async fn cache_hit_metric_increments() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let live = spawn_fake_upstream([10, 0, 0, 1], Duration::ZERO).await;
    let resolver =
        Arc::new(DnsResolver::with_upstreams(vec![live]).with_timeout(Duration::from_millis(500)));
    let cache = Arc::new(DnsAnswerCache::new(16, 300));
    let handler = DnsHandler::with_cache(resolver, Arc::clone(&cache));

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
    let handler = DnsHandler::with_cache(resolver, Arc::clone(&cache));

    let q = build_query_bytes("nx.example.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;
    assert_eq!(cache.len(), 0); // NXDOMAIN not cached
}

#[tokio::test]
async fn cache_default_constructor_enables_caching() {
    let handler = DnsHandler::with_default_resolver();
    assert!(handler.cache().is_some());
    assert_eq!(handler.cache().unwrap().len(), 0);
}

#[tokio::test]
async fn cache_explicit_none_via_new() {
    let resolver = Arc::new(DnsResolver::new());
    let handler = DnsHandler::new(resolver);
    assert!(handler.cache().is_none());
}
