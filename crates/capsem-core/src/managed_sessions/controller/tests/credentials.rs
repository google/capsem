use super::*;

#[tokio::test]
async fn credential_scope_belongs_to_exact_live_active_ownership_and_is_fenced_on_close() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    fixture.create_release.add_permits(1);
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([101; 32]);
    let snapshot = owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            Arc::new(fixture.clone()),
        )
        .await
        .unwrap();
    let scope = owner.credential_owner(&fixture.binding).await.unwrap().unwrap();
    assert_eq!(scope.generation(), snapshot.generation());
    assert!(scope.is_current());
    assert!(scope.remaining().unwrap() > Duration::from_secs(1));
    let replaced = VmBinding::new(fixture.binding.id().into(), capsem_crypto::uuid_v4()).unwrap();
    assert!(owner.credential_owner(&replaced).await.unwrap().is_none());
    let closing = owner.clone();
    let task = tokio::spawn(async move { closing.close(request, cap).await });
    tokio::time::timeout(Duration::from_secs(1), fixture.cleanup_entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(!scope.is_current());
    assert!(scope.remaining().is_none());
    assert!(owner.credential_owner(&fixture.binding).await.is_err());
    fixture.cleanup_release.add_permits(1);
    task.await.unwrap().unwrap();
    assert!(!scope.is_current());
}

#[tokio::test]
async fn captured_scope_observes_original_native_liveness_and_shorter_policy() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    fixture.create_release.add_permits(1);
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([102; 32]);
    owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            Arc::new(fixture.clone()),
        )
        .await
        .unwrap();
    let scope = owner.credential_owner(&fixture.binding).await.unwrap().unwrap();
    fixture.live.store(false, Ordering::SeqCst);
    assert!(!scope.is_current());
    fixture.live.store(true, Ordering::SeqCst);
    assert!(scope.is_current());
    owner
        .set_lease_limit(LeasePolicy::new(Duration::from_millis(40)).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), fixture.cleanup_entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(!scope.is_current());
    fixture.cleanup_release.add_permits(1);
    owner.close(request, cap).await.unwrap();
}
