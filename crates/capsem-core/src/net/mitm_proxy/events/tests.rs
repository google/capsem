use super::*;

#[test]
fn kind_layer_assignment() {
    assert_eq!(EventKind::RawRequestHead.layer(), EventLayer::L1);
    assert_eq!(EventKind::RawResponseChunk.layer(), EventLayer::L1);
    assert_eq!(EventKind::SseEvent.layer(), EventLayer::L2);
    assert_eq!(EventKind::JsonRpcMessage.layer(), EventLayer::L2);
    assert_eq!(EventKind::DnsQuery.layer(), EventLayer::L2);
    assert_eq!(EventKind::AiCallEnd.layer(), EventLayer::L3);
    assert_eq!(EventKind::McpCall.layer(), EventLayer::L3);
}

#[test]
fn layer_ordering_strict() {
    assert!(EventLayer::L1 < EventLayer::L2);
    assert!(EventLayer::L2 < EventLayer::L3);
    assert!(EventLayer::L1 < EventLayer::L3);
}

#[test]
fn mask_membership() {
    let m = EventMask::single(EventKind::SseEvent) | EventMask::single(EventKind::AiCallEnd);
    assert!(m.contains(EventKind::SseEvent));
    assert!(m.contains(EventKind::AiCallEnd));
    assert!(!m.contains(EventKind::DnsQuery));
    assert!(!m.contains(EventKind::RawRequestHead));
}

#[test]
fn mask_empty_and_all() {
    assert!(EventMask::empty().is_empty());
    assert!(!EventMask::all().is_empty());
    for k in [
        EventKind::RawRequestHead,
        EventKind::SseEvent,
        EventKind::McpCall,
    ] {
        assert!(EventMask::all().contains(k));
        assert!(!EventMask::empty().contains(k));
    }
}

#[test]
fn mask_from_kind_is_single() {
    let m: EventMask = EventKind::DnsQuery.into();
    assert!(m.contains(EventKind::DnsQuery));
    assert!(!m.contains(EventKind::DnsAnswer));
}

#[test]
fn event_kind_round_trips_through_event() {
    let mut chunk = bytes::Bytes::from_static(b"hello");
    let ev = Event::RawResponseChunk(&mut chunk);
    assert_eq!(ev.kind(), EventKind::RawResponseChunk);
    assert_eq!(ev.layer(), EventLayer::L1);

    let mut q = DnsQuery::default();
    let ev2 = Event::DnsQuery(&mut q);
    assert_eq!(ev2.kind(), EventKind::DnsQuery);
    assert_eq!(ev2.layer(), EventLayer::L2);
}

#[test]
fn raw_request_chunk_is_mutable() {
    // The credential-rewrite contract: hooks may rewrite a chunk in place
    // and even change its length by replacing the inner Bytes.
    let mut chunk = bytes::Bytes::from_static(b"__placeholder__");
    {
        let mut ev = Event::RawRequestChunk(&mut chunk);
        if let Event::RawRequestChunk(c) = &mut ev {
            **c = bytes::Bytes::from_static(b"sk-real-secret-AAAA");
        }
    }
    assert_eq!(&chunk[..], b"sk-real-secret-AAAA");
}
