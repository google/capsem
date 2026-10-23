use super::*;

#[test]
fn cancellation_names_one_nonzero_query_and_cannot_be_decoded_as_a_query() {
    let cancel = DnsQueryCancellation {
        cancel_query_id: NonZeroU32::new(17).unwrap(),
    };
    let frame = encode_dns_cancellation(&cancel).unwrap();
    assert!(
        matches!(decode_dns_session_message(&frame[4..]).unwrap(), DnsSessionMessage::Cancel(value) if value == cancel)
    );
    assert!(crate::decode_dns_request(&frame[4..]).is_err());
    let zero = rmp_serde::to_vec_named(&serde_json::json!({"cancel_query_id":0})).unwrap();
    assert!(decode_dns_session_message(&zero).is_err());
}

#[test]
fn existing_queries_keep_their_wire_bytes_and_decode_as_queries() {
    let request = crate::DnsRequest {
        id: 19,
        raw: vec![1, 2, 3],
        proto: "tcp".into(),
        process_name: None,
    };
    let frame = crate::encode_dns_request(&request).unwrap();
    assert!(
        matches!(decode_dns_session_message(&frame[4..]).unwrap(), DnsSessionMessage::Query(value) if value == request)
    );
    let legacy = rmp_serde::to_vec_named(&serde_json::json!({"raw":[], "proto":"udp"})).unwrap();
    assert!(matches!(decode_dns_session_message(&legacy).unwrap(), DnsSessionMessage::Query(value) if value.id == 0));
}
