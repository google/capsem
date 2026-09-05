//! DNS request/response frames on the vsock DNS port.

use super::*;

#[test]
fn roundtrip_dns_request() {
    let req = DnsRequest {
        id: 0,
        raw: vec![0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0],
        proto: "udp".into(),
        process_name: Some("curl".into()),
    };
    let frame = encode_dns_request(&req).unwrap();
    let len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]);
    assert!(len < MAX_FRAME_SIZE);
    let decoded = decode_dns_request(&frame[4..]).unwrap();
    assert_eq!(decoded, req);
}

#[test]
fn roundtrip_dns_request_no_process_name() {
    let req = DnsRequest {
        id: 0,
        raw: vec![0u8; 12],
        proto: "tcp".into(),
        process_name: None,
    };
    let frame = encode_dns_request(&req).unwrap();
    let decoded = decode_dns_request(&frame[4..]).unwrap();
    assert_eq!(decoded, req);
}

#[test]
fn roundtrip_dns_response() {
    let resp = DnsResponse {
        id: 0,
        raw: vec![0x12, 0x34, 0x81, 0x83, 0, 1, 0, 0, 0, 0, 0, 0],
        decision: "denied".into(),
        rcode: 3,
    };
    let frame = encode_dns_response(&resp).unwrap();
    let decoded = decode_dns_response(&frame[4..]).unwrap();
    assert_eq!(decoded, resp);
}

#[test]
fn dns_envelope_is_compact() {
    // 60-byte raw query + small metadata should fit comfortably
    // under 200 bytes encoded -- ensures we don't accidentally pull
    // in heavy framing (e.g. nested struct + named fields blow-up).
    let req = DnsRequest {
        id: 0,
        raw: vec![0u8; 60],
        proto: "udp".into(),
        process_name: None,
    };
    let frame = encode_dns_request(&req).unwrap();
    let payload_len = frame.len() - 4;
    assert!(
        payload_len < 200,
        "DnsRequest payload {payload_len} bytes, expected < 200"
    );
}

#[test]
fn decode_dns_request_garbage_fails() {
    assert!(decode_dns_request(&[0xFF, 0xFE]).is_err());
}

#[test]
fn decode_dns_response_garbage_fails() {
    assert!(decode_dns_response(&[0xFF, 0xFE]).is_err());
}

#[test]
fn dns_envelope_is_disjoint_from_ipc_frames() {
    // The DNS envelope is a freestanding RMP-encoded struct (NOT a
    // tagged-enum like HostToGuest / GuestToHost). It must NOT
    // accidentally trip the looks_like_ipc_frame heuristic, otherwise
    // a stray DNS frame leaked to a tty would be mis-flagged as a
    // control-channel leak. Spot-check that an encoded DnsRequest
    // does NOT match the fixmap[1] / fixmap[2] enum-frame shape.
    let req = DnsRequest {
        id: 0,
        raw: vec![0u8; 12],
        proto: "udp".into(),
        process_name: None,
    };
    let frame = encode_dns_request(&req).unwrap();
    // Skip the 4-byte length prefix; check the RMP body.
    assert!(
        !looks_like_ipc_frame(&frame[4..]),
        "DnsRequest accidentally matches the IPC enum frame shape"
    );
}

#[test]
fn dns_correlation_id_round_trips_in_both_directions() {
    let req = DnsRequest {
        id: 0xDEAD_BEEF,
        raw: vec![1, 2, 3],
        proto: "udp".into(),
        process_name: None,
    };
    let frame = encode_dns_request(&req).unwrap();
    assert_eq!(decode_dns_request(&frame[4..]).unwrap().id, 0xDEAD_BEEF);
    let resp = DnsResponse {
        id: u32::MAX,
        raw: vec![9],
        decision: "allowed".into(),
        rcode: 0,
    };
    let frame = encode_dns_response(&resp).unwrap();
    assert_eq!(decode_dns_response(&frame[4..]).unwrap().id, u32::MAX);
}

/// A peer built before the correlation id existed sends frames without it;
/// they decode with id 0, the value that means "answer in lock-step".
#[test]
fn dns_frames_without_a_correlation_id_decode_as_id_zero() {
    #[derive(serde::Serialize)]
    struct LegacyRequest {
        raw: Vec<u8>,
        proto: String,
        process_name: Option<String>,
    }
    #[derive(serde::Serialize)]
    struct LegacyResponse {
        raw: Vec<u8>,
        decision: String,
        rcode: u16,
    }
    let payload = rmp_serde::to_vec_named(&LegacyRequest {
        raw: vec![7],
        proto: "tcp".into(),
        process_name: None,
    })
    .unwrap();
    let req = decode_dns_request(&payload).unwrap();
    assert_eq!((req.id, req.raw.as_slice(), req.proto.as_str()), (0, &[7][..], "tcp"));
    let payload = rmp_serde::to_vec_named(&LegacyResponse {
        raw: vec![8],
        decision: "denied".into(),
        rcode: 3,
    })
    .unwrap();
    let resp = decode_dns_response(&payload).unwrap();
    assert_eq!((resp.id, resp.rcode), (0, 3));
}
