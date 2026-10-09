use super::*;
use crate::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration};

const POLICY_DIGEST: &str = "blake3:0000000000000000000000000000000000000000000000000000000000000001";

fn request_roundtrip(request: UpstreamGrantRequest) {
    let encoded = encode_upstream_grant_request(&request).unwrap();
    assert_eq!(decode_upstream_grant_request(&encoded).unwrap(), request);
}

fn response_roundtrip(response: UpstreamGrantResponse) {
    let encoded = encode_upstream_grant_response(&response).unwrap();
    assert_eq!(decode_upstream_grant_response(&encoded).unwrap(), response);
}

#[test]
fn every_request_round_trips_exactly() {
    for request in [
        UpstreamGrantRequest::ResolveTcp {
            request_id: 1,
            protocol: UpstreamProtocol::Tls,
            host: "api.example.com".into(),
            port: 443,
        },
        UpstreamGrantRequest::ResolveTcp {
            request_id: 2,
            protocol: UpstreamProtocol::Http,
            host: "2001:db8::1".into(),
            port: 8080,
        },
        UpstreamGrantRequest::ConnectTcp {
            request_id: 3,
            selection_id: 41,
        },
        UpstreamGrantRequest::OpenDns {
            request_id: 4,
            upstream_index: 0,
        },
        UpstreamGrantRequest::OpenLedger { request_id: 5 },
        UpstreamGrantRequest::AttachProxyTraffic {
            request_id: 6,
            service: ProxyTrafficService::Http,
        },
        UpstreamGrantRequest::AttachProxyTraffic {
            request_id: 7,
            service: ProxyTrafficService::Dns,
        },
        UpstreamGrantRequest::AttachProxyMcp { request_id: 8 },
        UpstreamGrantRequest::Adopted { grant_id: 51 },
        UpstreamGrantRequest::Release { resource_id: 61 },
        UpstreamGrantRequest::SetGuestMode {
            request_id: 7,
            relative_path: b"workspace/pyvenv/bin/activate".to_vec(),
            mode: 0o755,
        },
    ] {
        request_roundtrip(request);
    }
}

#[test]
fn every_response_round_trips_exactly() {
    for response in [
        UpstreamGrantResponse::TcpResolved {
            request_id: 1,
            selection_id: 11,
            protocol: UpstreamProtocol::Tls,
            judged_ip: Some("127.0.0.1".parse().unwrap()),
            policy_digest: POLICY_DIGEST.into(),
        },
        UpstreamGrantResponse::TcpResolved {
            request_id: 2,
            selection_id: 12,
            protocol: UpstreamProtocol::Http,
            judged_ip: Some("2001:db8::1".parse().unwrap()),
            policy_digest: POLICY_DIGEST.into(),
        },
        UpstreamGrantResponse::TcpResolved {
            request_id: 3,
            selection_id: 13,
            protocol: UpstreamProtocol::Http,
            judged_ip: None,
            policy_digest: POLICY_DIGEST.into(),
        },
        UpstreamGrantResponse::DescriptorGranted {
            request_id: 4,
            grant_id: 14,
            kind: UpstreamDescriptorKind::Tcp,
            policy_digest: POLICY_DIGEST.into(),
        },
        UpstreamGrantResponse::DescriptorGranted {
            request_id: 5,
            grant_id: 15,
            kind: UpstreamDescriptorKind::DnsUdp,
            policy_digest: POLICY_DIGEST.into(),
        },
        UpstreamGrantResponse::Denied {
            request_id: 6,
            reason: UpstreamGrantDenial::Revoked,
        },
        UpstreamGrantResponse::GuestModeSet { request_id: 7 },
        UpstreamGrantResponse::LedgerGranted {
            request_id: 8,
            grant: LedgerChannelGrant::new(LedgerGeneration::new([0x5a; 16]), 16, LedgerClientRole::VmOwner).unwrap(),
        },
        UpstreamGrantResponse::ProxyTrafficAdopted { request_id: 9 },
        UpstreamGrantResponse::ProxyMcpAdopted { request_id: 10 },
    ] {
        response_roundtrip(response);
    }
}

#[test]
fn guest_mode_paths_are_bounded_normalized_bytes() {
    request_roundtrip(UpstreamGrantRequest::SetGuestMode {
        request_id: 1,
        relative_path: vec![0xff, b'a'],
        mode: 0o700,
    });
    request_roundtrip(UpstreamGrantRequest::SetGuestMode {
        request_id: 2,
        relative_path: Vec::new(),
        mode: 0o755,
    });

    for relative_path in [
        b"/absolute".to_vec(),
        b"../escape".to_vec(),
        b"a/../escape".to_vec(),
        b"a//b".to_vec(),
        b"trailing/".to_vec(),
        b"nul\0byte".to_vec(),
        vec![b'x'; MAX_GUEST_SHARE_PATH_BYTES + 1],
    ] {
        assert!(encode_upstream_grant_request(&UpstreamGrantRequest::SetGuestMode {
            request_id: 3,
            relative_path,
            mode: 0o644,
        })
        .is_err());
    }
    assert!(encode_upstream_grant_request(&UpstreamGrantRequest::SetGuestMode {
        request_id: 4,
        relative_path: b"file".to_vec(),
        mode: 0o10_000,
    })
    .is_err());
}

#[test]
fn descriptor_expectations_are_explicit() {
    let request = UpstreamGrantRequest::OpenDns {
        request_id: 1,
        upstream_index: 0,
    };
    assert_eq!(request.expected_descriptor_count(), 0);
    assert_eq!(
        UpstreamGrantRequest::AttachProxyTraffic {
            request_id: 9,
            service: ProxyTrafficService::Dns,
        }
        .expected_descriptor_count(),
        1
    );
    assert_eq!(
        UpstreamGrantRequest::AttachProxyMcp { request_id: 10 }.expected_descriptor_count(),
        1
    );
    assert_eq!(
        UpstreamGrantResponse::ProxyTrafficAdopted { request_id: 9 }.expected_descriptor_count(),
        0
    );
    assert_eq!(
        UpstreamGrantResponse::ProxyMcpAdopted { request_id: 10 }.expected_descriptor_count(),
        0
    );
    assert_eq!(
        UpstreamGrantResponse::LedgerGranted {
            request_id: 2,
            grant: LedgerChannelGrant::new(LedgerGeneration::new([1; 16]), 3, LedgerClientRole::VmOwner).unwrap(),
        }
        .expected_descriptor_count(),
        1
    );
    assert_eq!(
        UpstreamGrantResponse::DescriptorGranted {
            request_id: 1,
            grant_id: 2,
            kind: UpstreamDescriptorKind::DnsUdp,
            policy_digest: POLICY_DIGEST.into(),
        }
        .expected_descriptor_count(),
        1
    );
    assert_eq!(
        UpstreamGrantResponse::Denied {
            request_id: 1,
            reason: UpstreamGrantDenial::NotAllowed,
        }
        .expected_descriptor_count(),
        0
    );
}

#[test]
fn ledger_grants_preserve_every_authorized_role() {
    for (index, role) in [
        LedgerClientRole::VmOwner,
        LedgerClientRole::Proxy,
        LedgerClientRole::Coordinator,
        LedgerClientRole::Reader,
        LedgerClientRole::Maintainer,
        LedgerClientRole::Supervisor,
    ]
    .into_iter()
    .enumerate()
    {
        response_roundtrip(UpstreamGrantResponse::LedgerGranted {
            request_id: 10 + index as u64,
            grant: LedgerChannelGrant::new(
                LedgerGeneration::new([u8::try_from(index + 1).unwrap(); 16]),
                20 + index as u64,
                role,
            )
            .unwrap(),
        });
    }
}

#[test]
fn malformed_ledger_grant_authority_is_rejected() {
    let response = UpstreamGrantResponse::LedgerGranted {
        request_id: 1,
        grant: LedgerChannelGrant::new(LedgerGeneration::new([0xab; 16]), 2, LedgerClientRole::VmOwner).unwrap(),
    };
    let valid = encode_upstream_grant_response(&response).unwrap();

    let mut zero_generation = valid;
    put_name(&mut zero_generation, "00000000000000000000000000000000").unwrap();
    assert!(decode_upstream_grant_response(&zero_generation).is_err());

    let mut uppercase_generation = valid;
    put_name(&mut uppercase_generation, "ABABABABABABABABABABABABABABABAB").unwrap();
    assert!(decode_upstream_grant_response(&uppercase_generation).is_err());

    let mut short_generation = valid;
    put_name(&mut short_generation, "abab").unwrap();
    assert!(decode_upstream_grant_response(&short_generation).is_err());

    let mut unknown_role = valid;
    put_u16(&mut unknown_role, DETAIL_RANGE, 99);
    assert!(decode_upstream_grant_response(&unknown_role).is_err());

    let mut unrelated_policy = valid;
    put_policy_digest(&mut unrelated_policy, POLICY_DIGEST).unwrap();
    assert!(decode_upstream_grant_response(&unrelated_policy).is_err());

    let mut generation_padding = valid;
    generation_padding[NAME_RANGE.start + 32] = 1;
    assert!(decode_upstream_grant_response(&generation_padding).is_err());
}

#[test]
fn request_encoding_rejects_noncanonical_or_oversized_hosts() {
    for host in [
        "Example.com".to_string(),
        "example.com.".to_string(),
        "example..com".to_string(),
        "[::1]".to_string(),
        "0:0:0:0:0:0:0:1".to_string(),
        "a".repeat(MAX_UPSTREAM_HOST_BYTES + 1),
    ] {
        assert!(encode_upstream_grant_request(&UpstreamGrantRequest::ResolveTcp {
            request_id: 1,
            protocol: UpstreamProtocol::Tls,
            host,
            port: 443,
        })
        .is_err());
    }
}

#[test]
fn maximum_host_fits_and_hostile_length_bytes_do_not_panic() {
    let host = ["a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(61)].join(".");
    assert_eq!(host.len(), MAX_UPSTREAM_HOST_BYTES);
    request_roundtrip(UpstreamGrantRequest::ResolveTcp {
        request_id: 1,
        protocol: UpstreamProtocol::Tls,
        host,
        port: 443,
    });

    let valid = encode_upstream_grant_request(&UpstreamGrantRequest::ConnectTcp {
        request_id: 2,
        selection_id: 3,
    })
    .unwrap();
    for hostile_length in [254, 255] {
        let mut malformed = valid;
        malformed[NAME_LENGTH_OFFSET] = hostile_length;
        assert!(decode_upstream_grant_request(&malformed).is_err());
    }
}

#[test]
fn zero_identifiers_and_port_are_rejected() {
    for request in [
        UpstreamGrantRequest::ResolveTcp {
            request_id: 0,
            protocol: UpstreamProtocol::Tls,
            host: "example.com".into(),
            port: 443,
        },
        UpstreamGrantRequest::ResolveTcp {
            request_id: 1,
            protocol: UpstreamProtocol::Tls,
            host: "example.com".into(),
            port: 0,
        },
        UpstreamGrantRequest::ConnectTcp {
            request_id: 1,
            selection_id: 0,
        },
        UpstreamGrantRequest::Adopted { grant_id: 0 },
        UpstreamGrantRequest::Release { resource_id: 0 },
    ] {
        assert!(encode_upstream_grant_request(&request).is_err(), "accepted {request:?}");
    }
}

#[test]
fn envelope_and_trailing_bytes_are_strict() {
    let valid = encode_upstream_grant_request(&UpstreamGrantRequest::ConnectTcp {
        request_id: 1,
        selection_id: 2,
    })
    .unwrap();
    for mutate in [
        |frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE]| frame[0] = b'X',
        |frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE]| frame[VERSION_OFFSET] = VERSION + 1,
        |frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE]| frame[RESERVED_RANGE.start] = 1,
        |frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE]| frame[NAME_RANGE.start + 1] = 1,
        |frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE]| frame[PATH_RANGE.start + 1] = 1,
    ] {
        let mut malformed = valid;
        mutate(&mut malformed);
        assert!(decode_upstream_grant_request(&malformed).is_err());
    }
}

#[test]
fn wrong_direction_and_unknown_kinds_are_rejected() {
    let request = encode_upstream_grant_request(&UpstreamGrantRequest::OpenDns {
        request_id: 1,
        upstream_index: 0,
    })
    .unwrap();
    assert!(decode_upstream_grant_response(&request).is_err());
    let response = encode_upstream_grant_response(&UpstreamGrantResponse::Denied {
        request_id: 1,
        reason: UpstreamGrantDenial::NotConfigured,
    })
    .unwrap();
    assert!(decode_upstream_grant_request(&response).is_err());
    let mut unknown = request;
    unknown[KIND_OFFSET] = 99;
    assert!(decode_upstream_grant_request(&unknown).is_err());
}

#[test]
fn malformed_protocol_reason_and_ownership_fields_are_rejected() {
    let mut resolve = encode_upstream_grant_request(&UpstreamGrantRequest::ResolveTcp {
        request_id: 1,
        protocol: UpstreamProtocol::Tls,
        host: "example.com".into(),
        port: 443,
    })
    .unwrap();
    put_u16(&mut resolve, DETAIL_RANGE, 99);
    assert!(decode_upstream_grant_request(&resolve).is_err());

    let mut denial = encode_upstream_grant_response(&UpstreamGrantResponse::Denied {
        request_id: 1,
        reason: UpstreamGrantDenial::Capacity,
    })
    .unwrap();
    put_u16(&mut denial, DETAIL_RANGE, 99);
    assert!(decode_upstream_grant_response(&denial).is_err());

    let mut adopted = encode_upstream_grant_request(&UpstreamGrantRequest::Adopted { grant_id: 7 }).unwrap();
    put_u64(&mut adopted, REQUEST_ID_RANGE, 8);
    assert!(decode_upstream_grant_request(&adopted).is_err());

    let mut released = encode_upstream_grant_request(&UpstreamGrantRequest::Release { resource_id: 9 }).unwrap();
    put_u16(&mut released, PORT_RANGE, 80);
    assert!(decode_upstream_grant_request(&released).is_err());
}

#[test]
fn response_requires_canonical_ip_and_resource_shape() {
    let mut resolved = encode_upstream_grant_response(&UpstreamGrantResponse::TcpResolved {
        request_id: 1,
        selection_id: 2,
        protocol: UpstreamProtocol::Tls,
        judged_ip: Some("::1".parse().unwrap()),
        policy_digest: POLICY_DIGEST.into(),
    })
    .unwrap();
    put_name(&mut resolved, "0:0:0:0:0:0:0:1").unwrap();
    assert!(decode_upstream_grant_response(&resolved).is_err());

    let mut granted = encode_upstream_grant_response(&UpstreamGrantResponse::DescriptorGranted {
        request_id: 1,
        grant_id: 3,
        kind: UpstreamDescriptorKind::Tcp,
        policy_digest: POLICY_DIGEST.into(),
    })
    .unwrap();
    put_name(&mut granted, "unexpected").unwrap();
    assert!(decode_upstream_grant_response(&granted).is_err());
}

#[test]
fn successful_responses_require_an_exact_policy_digest() {
    for policy_digest in [
        "policy-a",
        "blake3:000000000000000000000000000000000000000000000000000000000000000A",
        "blake3:000000000000000000000000000000000000000000000000000000000000001",
    ] {
        assert!(
            encode_upstream_grant_response(&UpstreamGrantResponse::DescriptorGranted {
                request_id: 1,
                grant_id: 2,
                kind: UpstreamDescriptorKind::DnsUdp,
                policy_digest: policy_digest.into(),
            })
            .is_err()
        );
    }

    let valid = encode_upstream_grant_response(&UpstreamGrantResponse::DescriptorGranted {
        request_id: 1,
        grant_id: 2,
        kind: UpstreamDescriptorKind::DnsUdp,
        policy_digest: POLICY_DIGEST.into(),
    })
    .unwrap();
    let mut missing = valid;
    missing[POLICY_DIGEST_RANGE].fill(0);
    assert!(decode_upstream_grant_response(&missing).is_err());

    let mut denied = encode_upstream_grant_response(&UpstreamGrantResponse::Denied {
        request_id: 1,
        reason: UpstreamGrantDenial::Revoked,
    })
    .unwrap();
    denied[POLICY_DIGEST_RANGE].copy_from_slice(&valid[POLICY_DIGEST_RANGE]);
    assert!(decode_upstream_grant_response(&denied).is_err());
}
