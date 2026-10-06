use super::super::super::{CacheState, ImageCache};
use super::super::tests::Registry;
use super::super::*;

#[tokio::test]
async fn request_transports_share_owner_proof_without_sharing_credentials() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let mut authenticated = registry.puller().with_cache(&owner);
    authenticated.authentication = RegistryAuth::Basic("account".into(), "request-secret".into());
    let image = authenticated.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let ready = authenticated.reconcile_cache(&key, parent.path()).await.unwrap();
    assert_eq!(ready.state, CacheState::Ready);
    drop(authenticated);
    let anonymous = registry.puller().with_cache(&owner);
    assert!(matches!(anonymous.authentication, RegistryAuth::Anonymous));
    assert_eq!(anonymous.cache_snapshot(&key).unwrap(), ready);
    assert_eq!(owner.snapshot(&key).unwrap(), ready);
    let lease = anonymous.cache.as_ref().unwrap().mutation_lease().await.unwrap();
    let invalid = owner.snapshot(&key).unwrap();
    assert!(invalid.verification_pending);
    assert!(invalid.epoch > ready.epoch);
    drop(lease);
    anonymous.reconcile_cache(&key, parent.path()).await.unwrap();
    let independent = ImageCache::at(root.path()).unwrap();
    assert!(
        independent.snapshot(&key).unwrap().verification_pending,
        "another owner cannot inherit a receipt's readiness"
    );
}
