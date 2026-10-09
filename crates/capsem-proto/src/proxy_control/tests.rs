use super::*;

const GENERATION: ProxyGeneration = ProxyGeneration::new([7; 16]);

#[test]
fn grants_round_trip_without_session_path_or_destination_authority() {
    for capability in [
        ProxyCapability::Traffic,
        ProxyCapability::Upstream,
        ProxyCapability::Credential,
        ProxyCapability::Ledger,
        ProxyCapability::PrivateNames,
        ProxyCapability::Mcp,
        ProxyCapability::Telemetry,
    ] {
        let grant = ProxyChannelGrant::new(GENERATION, 41, capability).unwrap();
        let request = ProxyControlRequest::Attach(grant);
        let frame = encode_proxy_control_request(request);
        assert_eq!(decode_proxy_control_request(&frame).unwrap(), request);
        let visible = String::from_utf8_lossy(&frame);
        assert!(!visible.contains("session"));
        assert!(!visible.contains("path"));
        assert!(!visible.contains("host"));
    }

    let shutdown = ProxyControlRequest::Shutdown { generation: GENERATION };
    assert_eq!(
        decode_proxy_control_request(&encode_proxy_control_request(shutdown)).unwrap(),
        shutdown
    );
}

#[test]
fn events_round_trip_generation_and_grant_identity() {
    for event in [
        ProxyControlEvent::Ready { generation: GENERATION },
        ProxyControlEvent::Adopted {
            generation: GENERATION,
            grant_id: 42,
        },
        ProxyControlEvent::Rejected {
            generation: GENERATION,
            grant_id: 43,
            reason: ProxyControlRejection::DuplicateCapability,
        },
        ProxyControlEvent::Closed {
            generation: GENERATION,
            grant_id: 44,
            reason: ProxyChannelCloseReason::Disconnected,
        },
        ProxyControlEvent::Stopped { generation: GENERATION },
    ] {
        assert_eq!(
            decode_proxy_control_event(&encode_proxy_control_event(event)).unwrap(),
            event
        );
    }
}

#[test]
fn malformed_or_stale_authority_records_fail_closed() {
    let grant = ProxyChannelGrant::new(GENERATION, 41, ProxyCapability::Upstream).unwrap();
    let valid = encode_proxy_control_request(ProxyControlRequest::Attach(grant));
    for (index, value) in [(0, 0), (3, 0), (4, 99), (5, 99)] {
        let mut frame = valid;
        frame[index] = value;
        assert!(decode_proxy_control_request(&frame).is_err());
    }
    let mut zero_generation = valid;
    zero_generation[GENERATION_RANGE].fill(0);
    assert!(decode_proxy_control_request(&zero_generation).is_err());
    let mut zero_grant = valid;
    zero_grant[GRANT_ID_RANGE].fill(0);
    assert!(decode_proxy_control_request(&zero_grant).is_err());
}
