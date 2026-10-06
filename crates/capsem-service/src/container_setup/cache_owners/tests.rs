use super::super::*;
use crate::tests::{insert_fake_instance_with_session_dir, make_test_state};

fn key(pin: char) -> capsem_assets::oci::CacheKey {
    capsem_assets::oci::CacheIdentity::new(
        &format!("registry.example/image@sha256:{}", pin.to_string().repeat(64)),
        stage::oci_architecture().unwrap(),
        capsem_assets::oci::RUNTIME_CONTRACT,
    )
    .unwrap()
    .key()
}

#[test]
fn cache_associations_refuse_stale_workload_and_replacement_vm_generations() {
    let state = make_test_state();
    let root = tempfile::tempdir().unwrap();
    insert_fake_instance_with_session_dir(&state, "box", 1, root.path().to_owned());
    let spawn = state.instances.lock().unwrap()["box"].generation;
    let image = cache_owners::CacheOwner {
        key: key('a'),
        vm: capsem_core::managed_sessions::VmBinding::new("box".into(), spawn).unwrap(),
    };
    let generation = state.containers.begin("box", "image");
    assert!(state.containers.active_cache_owners(&state).is_empty());
    assert!(state
        .containers
        .pin_image("box", generation, Some("manifest".into()), Some(image.clone())));
    assert_eq!(state.containers.active_cache_owners(&state), vec![image.clone()]);
    let changed = cache_owners::CacheOwner {
        key: key('b'),
        vm: image.vm.clone(),
    };
    assert!(!state
        .containers
        .pin_image("box", generation, Some("manifest".into()), Some(changed)));
    assert_eq!(state.containers.active_cache_owners(&state), vec![image.clone()]);
    let next = state.containers.begin("box", "replacement");
    assert!(!state
        .containers
        .pin_image("box", generation, Some("late".into()), Some(image.clone())));
    assert!(state.containers.active_cache_owners(&state).is_empty());
    assert!(state
        .containers
        .pin_image("box", next, Some("manifest".into()), Some(image.clone())));
    state.instances.lock().unwrap().get_mut("box").unwrap().generation = uuid::Uuid::new_v4();
    assert!(state.containers.active_cache_owners(&state).is_empty());
    state.containers.cancel("box");
    assert!(state.containers.active_cache_owners(&state).is_empty());
}

#[test]
fn cloned_and_restored_cache_identity_binds_only_the_new_instance_owner() {
    let state = make_test_state();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let clone = root.path().join("clone");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&clone).unwrap();
    let key = key('a');
    let record = LaunchRecord {
        image: "image".into(),
        digest: "native".into(),
        surface: None,
        resolved: None,
        manifest: Some("manifest".into()),
        cache_key: Some(key.as_str().into()),
    };
    capsem_foundation::unix::fs::atomic_write_private(
        &source.join(LAUNCH_RECORD),
        &serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    carry_launch_record(&source, &clone).unwrap();
    assert_eq!(
        read_launch_record(&clone).unwrap().cache_key.as_deref(),
        Some(key.as_str())
    );
    insert_fake_instance_with_session_dir(&state, "clone", 1, clone.clone());
    restore(&state, "clone");
    let first = state.containers.active_cache_owners(&state);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].key, key);
    assert_eq!(first[0].vm.id(), "clone");
    state.containers.cancel("clone");
    let generation = uuid::Uuid::new_v4();
    state.instances.lock().unwrap().get_mut("clone").unwrap().generation = generation;
    restore(&state, "clone");
    let next = state.containers.active_cache_owners(&state);
    assert_eq!(next[0].vm.generation(), generation);
    assert_ne!(next[0].vm.generation(), first[0].vm.generation());
    assert_eq!(next[0].key, key);
    let invalid = LaunchRecord {
        cache_key: Some("../../unowned".into()),
        ..record
    };
    capsem_foundation::unix::fs::atomic_write_private(
        &clone.join(LAUNCH_RECORD),
        &serde_json::to_vec(&invalid).unwrap(),
    )
    .unwrap();
    restore(&state, "clone");
    assert!(state.containers.active_cache_owners(&state).is_empty());
}

#[test]
fn launched_image_snapshot_is_consistent_and_legacy_has_no_invented_key() {
    let state = make_test_state();
    let root = tempfile::tempdir().unwrap();
    insert_fake_instance_with_session_dir(&state, "box", 1, root.path().to_owned());
    let generation = state.containers.begin("box", "image");
    state.containers.advance("box", generation, |status| {
        status.digest = Some("native".into());
        status.resolved = Some("pin".into());
    });
    let owner = cache_owners::CacheOwner {
        key: key('a'),
        vm: capsem_core::managed_sessions::VmBinding::new(
            "box".into(),
            state.instances.lock().unwrap()["box"].generation,
        )
        .unwrap(),
    };
    state
        .containers
        .pin_image("box", generation, Some("manifest".into()), Some(owner.clone()));
    record_launched(&state, "box").unwrap();
    let record = read_launch_record(root.path()).unwrap();
    assert_eq!(record.image, "image");
    assert_eq!(record.digest, "native");
    assert_eq!(record.manifest.as_deref(), Some("manifest"));
    assert_eq!(record.cache_key.as_deref(), Some(owner.key.as_str()));
    let replacement = state.containers.begin("box", "legacy");
    state
        .containers
        .pin_image("box", replacement, Some("legacy-manifest".into()), None);
    record_launched(&state, "box").unwrap();
    assert!(read_launch_record(root.path()).unwrap().cache_key.is_none());
    assert!(state.containers.active_cache_owners(&state).is_empty());
}
