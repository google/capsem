use super::*;

const GENERATION: LedgerGeneration = LedgerGeneration::new([9; 16]);

#[test]
fn requests_round_trip_authority_without_session_or_path() {
    for role in [
        LedgerClientRole::VmOwner,
        LedgerClientRole::Proxy,
        LedgerClientRole::Coordinator,
        LedgerClientRole::Reader,
        LedgerClientRole::Maintainer,
        LedgerClientRole::Supervisor,
    ] {
        let grant = LedgerChannelGrant::new(GENERATION, 71, role).unwrap();
        let frame = encode_ledger_control_request(LedgerControlRequest::Attach(grant));
        assert_eq!(
            decode_ledger_control_request(&frame).unwrap(),
            LedgerControlRequest::Attach(grant)
        );
        assert!(!String::from_utf8_lossy(&frame).contains("session"));
        assert!(!String::from_utf8_lossy(&frame).contains("path"));
    }
    let shutdown = LedgerControlRequest::Shutdown { generation: GENERATION };
    assert_eq!(
        decode_ledger_control_request(&encode_ledger_control_request(shutdown)).unwrap(),
        shutdown
    );
}

#[test]
fn events_round_trip_generation_identity_and_rejection() {
    for event in [
        LedgerControlEvent::Ready { generation: GENERATION },
        LedgerControlEvent::Adopted {
            generation: GENERATION,
            client_id: 72,
        },
        LedgerControlEvent::Rejected {
            generation: GENERATION,
            client_id: 73,
            reason: LedgerControlRejection::Capacity,
        },
        LedgerControlEvent::Rejected {
            generation: GENERATION,
            client_id: 74,
            reason: LedgerControlRejection::DuplicateClient,
        },
        LedgerControlEvent::Stopped { generation: GENERATION },
        LedgerControlEvent::Closed {
            generation: GENERATION,
            client_id: 75,
            reason: LedgerClientCloseReason::ProtocolError,
        },
    ] {
        assert_eq!(
            decode_ledger_control_event(&encode_ledger_control_event(event)).unwrap(),
            event
        );
    }
}

#[test]
fn malformed_records_fail_closed() {
    let grant = LedgerChannelGrant::new(GENERATION, 71, LedgerClientRole::Reader).unwrap();
    let valid = encode_ledger_control_request(LedgerControlRequest::Attach(grant));
    for (index, value) in [(0, 0), (3, 0), (4, 99), (5, 99)] {
        let mut frame = valid;
        frame[index] = value;
        assert!(decode_ledger_control_request(&frame).is_err());
    }
    let mut zero_generation = valid;
    zero_generation[GENERATION_RANGE].fill(0);
    assert!(decode_ledger_control_request(&zero_generation).is_err());
    let mut zero_client = valid;
    zero_client[CLIENT_ID_RANGE].fill(0);
    assert!(decode_ledger_control_request(&zero_client).is_err());
}
