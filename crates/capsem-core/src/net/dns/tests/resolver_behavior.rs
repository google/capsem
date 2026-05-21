use super::*;

#[tokio::test]
async fn upstream_unreachable_returns_servfail_with_decision_error() {
    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(150)),
    );
    let handler = DnsHandler::new(resolver);

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
    let handler = DnsHandler::new(resolver);

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
    let handler = DnsHandler::new(resolver);

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
    let handler = DnsHandler::new(resolver);

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
    let handler = DnsHandler::new(resolver);

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
        crate::net::dns::resolver::DEFAULT_UPSTREAMS.len()
    );
}
