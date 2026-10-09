use super::*;

#[test]
fn requests_round_trip_with_bounded_identity() {
    for request in [
        GatewayGrantRequest::OpenService { request_id: 7 },
        GatewayGrantRequest::OpenOwnerHandoff {
            request_id: u64::MAX,
            vm_id: "session-1".into(),
        },
    ] {
        let frame = encode_gateway_grant_request(&request).unwrap();
        assert_eq!(decode_gateway_grant_request(&frame).unwrap(), request);
    }
    assert!(encode_gateway_grant_request(&GatewayGrantRequest::OpenOwnerHandoff {
        request_id: 1,
        vm_id: "x".repeat(MAX_GATEWAY_VM_ID_BYTES + 1),
    })
    .is_err());
}

#[test]
fn responses_round_trip_with_no_worker_chosen_path() {
    for response in [
        GatewayGrantResponse::Granted {
            request_id: 9,
            kind: GatewayGrantKind::Service,
        },
        GatewayGrantResponse::Denied {
            request_id: 10,
            reason: GatewayGrantDenial::Revoked,
        },
    ] {
        let frame = encode_gateway_grant_response(response);
        assert_eq!(decode_gateway_grant_response(&frame).unwrap(), response);
    }
}

#[test]
fn malformed_and_extended_records_are_refused() {
    let request = GatewayGrantRequest::OpenService { request_id: 1 };
    for offset in [0, 2, RESERVED_RANGE.start] {
        let mut frame = encode_gateway_grant_request(&request).unwrap();
        frame[offset] ^= 1;
        assert!(
            decode_gateway_grant_request(&frame).is_err(),
            "accepted corruption at {offset}"
        );
    }
    let mut response = encode_gateway_grant_response(GatewayGrantResponse::Granted {
        request_id: 1,
        kind: GatewayGrantKind::Service,
    });
    response[VM_ID_LENGTH_OFFSET] = 1;
    assert!(decode_gateway_grant_response(&response).is_err());
}
