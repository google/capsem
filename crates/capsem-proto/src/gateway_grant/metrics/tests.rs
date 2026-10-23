use super::*;

#[test]
fn binary_body_round_trip_has_only_fixed_generation_and_ordered_request_authority() {
    let request = GatewayMetricRequest {
        generation: [7; 16],
        request_id: 1,
        body: vec![0, 0xff, 7],
    };
    let encoded = rmp_serde::to_vec_named(&request).unwrap();
    let decoded = rmp_serde::from_slice::<GatewayMetricRequest>(&encoded).unwrap();
    assert_eq!(request, decoded);
    assert!(decoded.valid_for([7; 16], 1));
    assert!(!decoded.valid_for([8; 16], 1));
    assert!(!decoded.valid_for([7; 16], 2));
    assert!(!decoded.valid_for([7; 16], 0));
    assert!(!format!("{request:?}").contains("255"));
    assert_eq!(
        MAX_GATEWAY_METRIC_FRAME_BYTES,
        MAX_GATEWAY_METRIC_BODY_BYTES as u32 + 256
    );
}

#[test]
fn metadata_receipts_have_no_collector_response_body_or_destination() {
    for message in [
        GatewayMetricMessage::Hello { generation: [1; 16] },
        GatewayMetricMessage::Receipt {
            request_id: 1,
            outcome: GatewayMetricOutcome::Accepted,
        },
        GatewayMetricMessage::Receipt {
            request_id: 1,
            outcome: GatewayMetricOutcome::Rejected,
        },
        GatewayMetricMessage::Receipt {
            request_id: 1,
            outcome: GatewayMetricOutcome::Unavailable,
        },
    ] {
        let encoded = rmp_serde::to_vec_named(&message).unwrap();
        assert_eq!(
            rmp_serde::from_slice::<GatewayMetricMessage>(&encoded).unwrap(),
            message
        );
        assert!(encoded.len() <= 96);
    }
}

#[test]
fn rejects_empty_overflow_zero_identity_and_unknown_authority_fields() {
    let mut request = GatewayMetricRequest {
        generation: [1; 16],
        request_id: 1,
        body: vec![1],
    };
    request.body.clear();
    assert!(!request.valid_for([1; 16], 1));
    request.body = vec![0; MAX_GATEWAY_METRIC_BODY_BYTES + 1];
    assert!(!request.valid_for([1; 16], 1));
    request.body = vec![1];
    request.generation = [0; 16];
    assert!(!request.valid_for([0; 16], 1));
    let mut encoded = rmp_serde::to_vec_named(&request).unwrap();
    assert_eq!(encoded[0], 0x83); // Three named fields.
    encoded[0] = 0x84;
    encoded.extend_from_slice(&[0xa3, b'u', b'r', b'l', 0xa1, b'x']);
    assert!(rmp_serde::from_slice::<GatewayMetricRequest>(&encoded).is_err());
}
