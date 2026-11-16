use super::*;

const ACCESS: [u8; 16] = [
    0xfc, 0x12, 0x07, 0x82, 0x22, 0x72, 0x4b, 0x3d, 0xa2, 0xe6, 0xa2, 0x6e, 0xe2, 0xf8, 0xab, 0x61,
];

fn requests() -> [GatewayAuthRequest; 2] {
    [
        GatewayAuthRequest::RegisterOperator {
            request_id: 1,
            token: GatewayAuthSecret::from_bytes(std::array::from_fn(|index| b'a' + (index % 26) as u8)),
        },
        GatewayAuthRequest::AuthenticateAgent {
            request_id: 2,
            access_id: ACCESS,
            capability: GatewayAuthSecret::from_bytes(std::array::from_fn(|index| index as u8)),
        },
    ]
}

#[test]
fn existing_capability_proofs_roundtrip_without_json_or_frame_growth() {
    for request in requests() {
        let frame = encode_gateway_auth_request(&request).unwrap();
        assert_eq!(frame.len(), super::super::GATEWAY_GRANT_FRAME_SIZE);
        assert!(is_gateway_auth_frame(&frame));
        assert_eq!(decode_gateway_auth_request(&frame).unwrap(), request);
        assert!(decode_gateway_auth_response(&frame).is_err());
        assert!(super::super::decode_gateway_grant_request(&frame).is_err());
    }
    for response in [
        GatewayAuthResponse::RegisteredOperator { request_id: 1 },
        GatewayAuthResponse::AuthenticatedAgent {
            request_id: 2,
            access_id: ACCESS,
        },
        GatewayAuthResponse::Denied {
            request_id: 3,
            reason: GatewayAuthDenial::Forbidden,
        },
        GatewayAuthResponse::Denied {
            request_id: 4,
            reason: GatewayAuthDenial::Unavailable,
        },
    ] {
        let frame = encode_gateway_auth_response(response).unwrap();
        assert_eq!(decode_gateway_auth_response(&frame).unwrap(), response);
        assert!(decode_gateway_auth_request(&frame).is_err());
        assert!(super::super::decode_gateway_grant_response(&frame).is_err());
    }
}

#[test]
fn secret_debug_is_redacted_and_does_not_serialize_an_actor_claim() {
    for request in requests() {
        let debug = format!("{request:?}");
        assert!(debug.contains("GatewayAuthSecret([REDACTED])"));
        assert!(!debug.contains(&"a".repeat(32)));
        assert!(!debug.contains("97, 97"));
    }
}

#[test]
fn corrupt_padding_identity_direction_and_version_are_refused() {
    let [operator, agent] = requests();
    for request in [operator, agent] {
        let frame = encode_gateway_auth_request(&request).unwrap();
        for offset in [0, 2, 3, 12, 13, 78, 79] {
            let mut corrupted = frame;
            corrupted[offset] ^= 0xff;
            assert!(decode_gateway_auth_request(&corrupted).is_err(), "offset {offset}");
        }
    }
    let response = encode_gateway_auth_response(GatewayAuthResponse::AuthenticatedAgent {
        request_id: 2,
        access_id: ACCESS,
    })
    .unwrap();
    for offset in [0, 2, 3, 12, 13, 30, 77, 78, 79] {
        let mut corrupted = response;
        corrupted[offset] ^= 0xff;
        assert!(decode_gateway_auth_response(&corrupted).is_err(), "offset {offset}");
    }
    let invalid = GatewayAuthRequest::AuthenticateAgent {
        request_id: 2,
        access_id: [0; 16],
        capability: GatewayAuthSecret::from_bytes([0; 32]),
    };
    assert!(encode_gateway_auth_request(&invalid).is_err());
    let invalid = GatewayAuthRequest::RegisterOperator {
        request_id: 1,
        token: GatewayAuthSecret::from_bytes([b' '; 64]),
    };
    assert!(encode_gateway_auth_request(&invalid).is_err());
    let old =
        super::super::encode_gateway_grant_request(&super::super::GatewayGrantRequest::OpenService { request_id: 1 })
            .unwrap();
    assert!(!is_gateway_auth_frame(&old));
    assert!(decode_gateway_auth_request(&old).is_err());
}

#[test]
fn malformed_raw_proofs_are_rejected_before_any_authority_lookup() {
    let [operator, agent] = requests();
    let mut operator_frame = encode_gateway_auth_request(&operator).unwrap();
    operator_frame[14] = b' ';
    assert!(decode_gateway_auth_request(&operator_frame).is_err());
    let mut agent_frame = encode_gateway_auth_request(&agent).unwrap();
    agent_frame[14 + 6] = 0; // Invalid UUID version in a correctly sized frame.
    assert!(decode_gateway_auth_request(&agent_frame).is_err());
    let mut response = encode_gateway_auth_response(GatewayAuthResponse::AuthenticatedAgent {
        request_id: 2,
        access_id: ACCESS,
    })
    .unwrap();
    response[14 + 8] = 0; // Invalid UUID variant.
    assert!(decode_gateway_auth_response(&response).is_err());
    let mut zero_id = encode_gateway_auth_request(&agent).unwrap();
    zero_id[4..12].fill(0);
    assert!(decode_gateway_auth_request(&zero_id).is_err());
}
