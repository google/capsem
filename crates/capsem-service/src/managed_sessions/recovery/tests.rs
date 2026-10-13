use super::*;
use capsem_core::managed_sessions::{Capability, Registry, Reservation};

#[tokio::test]
async fn unprepared_recovery_cannot_adopt_foreign_tickets_or_intents_or_clean_a_vm() {
    let directory = tempfile::tempdir().unwrap();
    let registry = Registry::open(&directory.path().join("ownership")).unwrap();
    let token = Capability::from_bytes([9; 32]);
    let request = capsem_crypto::uuid_v4();
    let Reservation::New(ticket) = registry.reserve(request, &token).unwrap() else {
        panic!()
    };
    let state = crate::tests::make_test_state();
    let snapshot = registry.inspect(request, &token).unwrap().unwrap();
    let authority = capsem_credentials::GrantAuthority::new(capsem_credentials::GrantLimits {
        max_grants: 1,
        max_lifetime: std::time::Duration::from_secs(300),
    })
    .unwrap();
    let owner = effects(state.clone(), &snapshot, Arc::new(authority)).unwrap();
    assert!(owner.reconcile(ticket.clone(), None).await.unwrap().is_none());
    let binding = VmBinding::new(new_persistent_vm_id(), capsem_crypto::uuid_v4()).unwrap();
    assert!(owner.reconcile(ticket.clone(), Some(binding.clone())).await.is_err());
    assert!(owner.is_live(&binding).is_err());
    assert!(owner.cleanup(ticket, binding).await.is_err());
    let Reservation::New(foreign) = registry.reserve(capsem_crypto::uuid_v4(), &token).unwrap() else {
        panic!()
    };
    assert!(owner.reconcile(foreign, None).await.is_err());
    let other = Registry::open(&directory.path().join("other")).unwrap();
    let Reservation::New(foreign_generation) = other.reserve(request, &token).unwrap() else {
        panic!()
    };
    assert!(owner.reconcile(foreign_generation, None).await.is_err());
    assert!(state.instances.lock().unwrap().is_empty());
}
