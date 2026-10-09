use super::*;
use crate::tests::make_test_state;
use capsem_core::managed_sessions::{Capability, LeaseClock, LeasePolicy, Registry, Reservation};
use std::time::{Duration, Instant};

fn authority() -> Arc<capsem_credentials::GrantAuthority> {
    Arc::new(
        capsem_credentials::GrantAuthority::new(capsem_credentials::GrantLimits {
            max_grants: 8,
            max_lifetime: Duration::from_secs(30),
        })
        .unwrap(),
    )
}

#[tokio::test]
async fn broker_retirement_accepts_only_the_original_service_scope_and_spawn() {
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::open(&root.path().join("ownership")).unwrap();
    let capability = Capability::from_bytes([42; 32]);
    let Reservation::New(ticket) = registry.reserve(uuid::Uuid::new_v4(), &capability).unwrap() else {
        panic!()
    };
    let Reservation::New(other) = registry.reserve(uuid::Uuid::new_v4(), &capability).unwrap() else {
        panic!()
    };
    let lifecycle =
        super::super::ManagedLifecycle::new_with_broker(make_test_state(), ProvisionRequest::default(), authority())
            .unwrap();
    let binding = lifecycle.binding.clone();
    assert!(
        lifecycle.grants.revoke(ticket.clone(), binding.clone()).await.is_err(),
        "an unbound continuation has no retirement authority"
    );
    lifecycle.scope(&ticket).unwrap();
    let expected = capsem_credentials::GrantSession::new(
        *uuid::Uuid::parse_str(binding.id()).unwrap().as_bytes(),
        *ticket.generation().as_bytes(),
        *binding.generation().as_bytes(),
    )
    .unwrap();
    assert_eq!(bound_session(&ticket, &binding, &lifecycle.work).unwrap(), expected);
    assert!(
        lifecycle.credential_binding(&ticket).is_err(),
        "a reserved UUID without a tracked owner cannot issue a grant"
    );
    assert!(lifecycle.grants.revoke(other, binding.clone()).await.is_err());
    let wrong_spawn = VmBinding::new(binding.id().into(), uuid::Uuid::new_v4()).unwrap();
    assert!(lifecycle.grants.revoke(ticket.clone(), wrong_spawn).await.is_err());
    lifecycle.grants.revoke(ticket.clone(), binding.clone()).await.unwrap();
    lifecycle.grants.revoke(ticket, binding).await.unwrap();
}

#[tokio::test]
async fn recovery_broker_scope_comes_from_the_actual_durable_spawn_receipt() {
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::open(&root.path().join("ownership")).unwrap();
    let capability = Capability::from_bytes([42; 32]);
    let Reservation::New(ticket) = registry.reserve(uuid::Uuid::new_v4(), &capability).unwrap() else {
        panic!()
    };
    let clock = LeaseClock::new(100, Instant::now());
    registry
        .start_lease(&ticket, LeasePolicy::new(Duration::from_secs(10)).unwrap(), clock)
        .unwrap();
    registry.begin_create(&ticket, clock).unwrap();
    let binding = VmBinding::new(uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4()).unwrap();
    let snapshot = registry.prepare_spawn(&ticket, binding.clone(), clock).unwrap();
    let recovery =
        super::super::ManagedLifecycle::for_recovery_with_broker(make_test_state(), &snapshot, authority()).unwrap();
    assert!(recovery.request.is_none());
    assert_eq!(recovery.binding, binding);
    recovery.grants.revoke(ticket.clone(), binding).await.unwrap();
    let invalid = VmBinding::new("caller-label".into(), uuid::Uuid::new_v4()).unwrap();
    assert!(bound_session(&ticket, &invalid, &recovery.work).is_err());
}
