use serde::{Deserialize, Serialize};

use super::*;

const GENERATION: LedgerGeneration = LedgerGeneration::new([7; 16]);

fn grant(role: LedgerClientRole) -> LedgerChannelGrant {
    LedgerChannelGrant::new(GENERATION, 41, role).expect("valid grant")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum TestOperation {
    Admit,
    Read,
}

impl LedgerOperation for TestOperation {
    fn capability(&self) -> LedgerCapability {
        match self {
            Self::Admit => LedgerCapability::Admit,
            Self::Read => LedgerCapability::Read,
        }
    }
}

#[test]
fn named_messagepack_round_trip_carries_only_generation_identity() {
    let hello = LedgerHello::for_grant(&grant(LedgerClientRole::Proxy));
    let encoded = rmp_serde::to_vec_named(&hello).expect("encode hello");
    let value: serde_json::Value = rmp_serde::from_slice(&encoded).expect("decode named map");
    let keys = value
        .as_object()
        .expect("named map")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(keys, ["client_id", "generation", "protocol_version", "schema_hash"]);
    assert_eq!(rmp_serde::from_slice::<LedgerHello>(&encoded).unwrap(), hello);
    assert!(!String::from_utf8_lossy(&encoded).contains("session"));
    assert!(!String::from_utf8_lossy(&encoded).contains("path"));
}

#[test]
fn server_and_client_reject_wrong_generation() {
    let expected = grant(LedgerClientRole::VmOwner);
    let wrong = LedgerChannelGrant::new(LedgerGeneration::new([9; 16]), 41, LedgerClientRole::VmOwner).unwrap();
    assert_eq!(
        expected.validate_hello(&LedgerHello::for_grant(&wrong)).unwrap_err(),
        LedgerHandshakeError::Generation
    );
    assert_eq!(
        expected
            .validate_welcome(&LedgerWelcome::for_grant(&wrong))
            .unwrap_err(),
        LedgerHandshakeError::Generation
    );
}

#[test]
fn role_rejects_wrong_operation_before_dispatch() {
    let request = LedgerRequest::new(1, TestOperation::Read).unwrap();
    assert_eq!(
        request.authorize(&grant(LedgerClientRole::Proxy)).unwrap_err(),
        LedgerProtocolError::Unauthorized {
            role: LedgerClientRole::Proxy,
            capability: LedgerCapability::Read,
        }
    );
    request.authorize(&grant(LedgerClientRole::Reader)).unwrap();
}

#[test]
fn only_maintainers_can_snapshot_a_ledger() {
    for role in [
        LedgerClientRole::VmOwner,
        LedgerClientRole::Proxy,
        LedgerClientRole::Coordinator,
        LedgerClientRole::Reader,
        LedgerClientRole::Supervisor,
    ] {
        assert!(!role.permits(LedgerCapability::Snapshot), "{role:?}");
    }
    assert!(LedgerClientRole::Maintainer.permits(LedgerCapability::Snapshot));
}

#[test]
fn compatibility_mismatch_fails_closed() {
    let expected = grant(LedgerClientRole::Reader);
    let mut hello = LedgerHello::for_grant(&expected);
    hello.protocol_version += 1;
    assert!(matches!(
        expected.validate_hello(&hello),
        Err(LedgerHandshakeError::ProtocolVersion { .. })
    ));
    let mut hello = LedgerHello::for_grant(&expected);
    hello.schema_hash ^= 1;
    assert!(matches!(
        expected.validate_hello(&hello),
        Err(LedgerHandshakeError::SchemaHash { .. })
    ));
}

#[test]
fn malformed_grants_and_oversized_failures_are_rejected() {
    assert_eq!(
        LedgerChannelGrant::new(LedgerGeneration::new([0; 16]), 1, LedgerClientRole::Reader).unwrap_err(),
        LedgerProtocolError::ZeroGeneration
    );
    assert_eq!(
        LedgerChannelGrant::new(GENERATION, 0, LedgerClientRole::Reader).unwrap_err(),
        LedgerProtocolError::ZeroClientId
    );
    assert_eq!(
        LedgerFailure::new(LedgerFailureCode::Storage, "x".repeat(MAX_LEDGER_ERROR_BYTES + 1)).unwrap_err(),
        LedgerProtocolError::ErrorMessageTooLong
    );
}

#[test]
fn response_round_trip_keeps_request_correlation_and_named_outcome() {
    let response = LedgerResponse::success(73, "durable".to_owned()).unwrap();
    let encoded = rmp_serde::to_vec_named(&response).unwrap();
    let decoded: LedgerResponse<String> = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(decoded, response);
    assert!(String::from_utf8_lossy(&encoded).contains("success"));
}
