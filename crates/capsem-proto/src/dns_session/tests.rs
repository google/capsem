use super::*;

#[test]
fn queries_reject_missing_and_zero_correlation_ids() {
    #[derive(Serialize)]
    struct InvalidQuery<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<u32>,
        #[serde(with = "serde_bytes")]
        raw: Vec<u8>,
        proto: &'a str,
    }
    #[derive(Serialize)]
    enum InvalidMessage<'a> {
        Query(InvalidQuery<'a>),
    }
    for id in [None, Some(0)] {
        let payload = rmp_serde::to_vec_named(&InvalidMessage::Query(InvalidQuery {
            id,
            raw: vec![1, 2],
            proto: "tcp",
        }))
        .unwrap();
        assert!(
            decode_dns_session_message(&payload).is_err(),
            "invalid query ID {id:?} was admitted"
        );
    }
    let query = crate::DnsRequest {
        id: 0,
        raw: vec![1, 2],
        proto: "tcp".into(),
        process_name: None,
    };
    assert!(crate::encode_dns_request(&query).is_err());
}

#[test]
fn responses_reject_missing_and_zero_correlation_ids() {
    #[derive(Serialize)]
    struct InvalidResponse {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<u32>,
        #[serde(with = "serde_bytes")]
        raw: Vec<u8>,
        decision: &'static str,
        rcode: u16,
    }
    #[derive(Serialize)]
    enum InvalidMessage {
        Answer(InvalidResponse),
    }
    for id in [None, Some(0)] {
        let payload = rmp_serde::to_vec_named(&InvalidMessage::Answer(InvalidResponse {
            id,
            raw: vec![1, 2],
            decision: "allowed",
            rcode: 0,
        }))
        .unwrap();
        assert!(crate::decode_dns_response(&payload).is_err());
    }
    assert!(crate::encode_dns_response(&crate::DnsResponse {
        id: 0,
        raw: vec![],
        decision: "error".into(),
        rcode: 2
    })
    .is_err());
}

#[test]
fn a_pending_receipt_is_bound_to_the_original_query_identity_and_expiry_not_an_answer() {
    let registration = crate::approval::ApprovalRegistration {
        ask_id: "abcdef012345".into(),
        event_id: "012345abcdef".into(),
        expires_at_unix_ms: 123456,
    };
    let frame = crate::encode_dns_pending(NonZeroU32::new(71).unwrap(), &registration).unwrap();
    assert!(
        matches!(crate::decode_dns_session_reply(&frame[4..]).unwrap(), DnsSessionReply::Pending {id,registration:receipt}
        if id.get()==71 && receipt==registration)
    );
    assert!(
        crate::decode_dns_response(&frame[4..]).is_err(),
        "a receipt cannot release query bytes"
    );
    for expires_at_unix_ms in [0, -1] {
        let invalid = crate::approval::ApprovalRegistration {
            expires_at_unix_ms,
            ..registration.clone()
        };
        assert!(crate::encode_dns_pending(NonZeroU32::new(71).unwrap(), &invalid).is_err());
    }
}

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
    #[derive(Serialize)]
    struct InvalidCancellation {
        cancel_query_id: u32,
    }
    #[derive(Serialize)]
    enum InvalidMessage {
        Cancel(InvalidCancellation),
    }
    let zero = rmp_serde::to_vec_named(&InvalidMessage::Cancel(InvalidCancellation { cancel_query_id: 0 })).unwrap();
    assert!(decode_dns_session_message(&zero).is_err());
}

#[test]
fn query_frames_are_explicitly_typed_and_borrow_original_bytes() {
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
    let untyped = rmp_serde::to_vec_named(&request).unwrap();
    assert!(
        decode_dns_session_message(&untyped).is_err(),
        "untyped legacy envelopes must fail closed"
    );
}
