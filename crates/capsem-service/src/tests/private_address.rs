//! One lifetime private address per VM: leased at create, held by the
//! running instance or the registry entry, released when neither holds it.
use super::*;
use std::net::Ipv4Addr;

fn pool_in_use(state: &ServiceState) -> usize {
    state.private_addresses.lock().unwrap().in_use()
}

#[test]
fn a_lease_dropped_before_commit_goes_back_to_the_pool() {
    let state = make_test_state();
    let before = pool_in_use(&state);
    {
        let lease = state.lease_private_address().unwrap();
        assert_eq!(pool_in_use(&state), before + 1);
        drop(lease);
    }
    assert_eq!(pool_in_use(&state), before, "an early return releases the address");
    let lease = state.lease_private_address().unwrap();
    let address = lease.commit();
    assert_eq!(pool_in_use(&state), before + 1);
    assert!(state.private_addresses.lock().unwrap().release(address));
}

#[test]
fn evicting_an_ephemeral_instance_releases_and_a_persistent_one_keeps() {
    let state = make_test_state();
    insert_fake_instance(&state, "ephemeral-vm", 4242);
    insert_fake_instance(&state, "named-vm", 4243);
    state.instances.lock().unwrap().get_mut("named-vm").unwrap().persistent = true;
    let named_address = state.instances.lock().unwrap()["named-vm"].private_address;
    assert_eq!(pool_in_use(&state), 2);

    let evicted = state.evict_instance("ephemeral-vm").expect("instance was running");
    assert!(!evicted.persistent);
    assert_eq!(pool_in_use(&state), 1, "the ephemeral address is free again");

    state.evict_instance("named-vm").expect("instance was running");
    assert_eq!(pool_in_use(&state), 1, "a stopped persistent VM keeps its address");
    assert_eq!(
        state.private_addresses.lock().unwrap().reserve(named_address),
        Err(capsem_core::net::address_pool::AddressError::InUse { address: named_address })
    );
    assert!(state.evict_instance("named-vm").is_none(), "eviction is idempotent");
}

#[test]
fn forgetting_a_persistent_entry_releases_its_address() {
    let (state, _dir) = make_test_state_with_tempdir();
    let address = state.lease_private_address().unwrap().commit();
    let mut entry = test_persistent_entry("kept", state.run_dir.join("persistent/kept"));
    entry.private_address = Some(address);
    state.persistent_registry.lock().unwrap().register(entry).unwrap();
    assert_eq!(pool_in_use(&state), 1);

    state.forget_persistent_entry("kept").unwrap();
    assert_eq!(pool_in_use(&state), 0);
    assert!(state.persistent_registry.lock().unwrap().get("kept").is_none());
    state.forget_persistent_entry("kept").unwrap();
    assert_eq!(pool_in_use(&state), 0, "forgetting twice is harmless");
}

#[test]
fn registry_addresses_are_reserved_at_startup_and_conflicts_dropped() {
    let (state, _dir) = make_test_state_with_tempdir();
    let path = state.run_dir.join("persistent_registry.json");
    let mut registry = PersistentRegistry::load(path.clone()).unwrap();
    let mut first = test_persistent_entry("first", state.run_dir.join("persistent/first"));
    first.private_address = Some(Ipv4Addr::new(10, 128, 0, 9));
    let mut duplicate = test_persistent_entry("duplicate", state.run_dir.join("persistent/duplicate"));
    duplicate.private_address = Some(Ipv4Addr::new(10, 128, 0, 9));
    duplicate.created_at = "1".into();
    let mut foreign = test_persistent_entry("foreign", state.run_dir.join("persistent/foreign"));
    foreign.private_address = Some(Ipv4Addr::new(192, 168, 1, 9));
    let legacy = test_persistent_entry("legacy", state.run_dir.join("persistent/legacy"));
    for entry in [first, duplicate, foreign, legacy] {
        registry.register(entry).unwrap();
    }

    let mut reloaded = PersistentRegistry::load(path).unwrap();
    let mut allocator = capsem_core::net::address_pool::AddressAllocator::new(capsem_config::PrivatePool::DEFAULT);
    crate::private_address::reserve_registry_addresses(&mut reloaded, &mut allocator);

    assert_eq!(allocator.in_use(), 1, "one reservation survives");
    assert_eq!(
        reloaded.get("first").unwrap().private_address,
        Some(Ipv4Addr::new(10, 128, 0, 9))
    );
    assert_eq!(
        reloaded.get("duplicate").unwrap().private_address,
        None,
        "second claimant loses"
    );
    assert_eq!(
        reloaded.get("foreign").unwrap().private_address,
        None,
        "outside the pool"
    );
    assert_eq!(reloaded.get("legacy").unwrap().private_address, None);
    assert_ne!(
        allocator.allocate().unwrap(),
        Ipv4Addr::new(10, 128, 0, 9),
        "a reserved address is never handed out again"
    );
}

#[tokio::test]
async fn list_and_info_report_the_private_address_of_running_and_stopped_vms() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "running-vm", 4242);
    let running_address = state.instances.lock().unwrap()["running-vm"].private_address;
    let stopped_dir = state.run_dir.join("persistent/stopped-vm");
    capsem_core::create_virtiofs_session(&stopped_dir, 64).unwrap();
    let mut stopped = test_persistent_entry("stopped-vm", stopped_dir);
    stopped.private_address = Some(Ipv4Addr::new(10, 128, 0, 77));
    let stopped_id = stopped.id.clone();
    state.persistent_registry.lock().unwrap().register(stopped).unwrap();

    let (status, list) = route_request(
        build_service_router(Arc::clone(&state)),
        axum::http::Method::GET,
        "/vms/list",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let by_id: std::collections::HashMap<String, serde_json::Value> = list["sandboxes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["id"].as_str().unwrap().to_string(), row["private_address"].clone()))
        .collect();
    assert_eq!(by_id["running-vm"], json!(running_address.to_string()));
    assert_eq!(by_id[&stopped_id], json!("10.128.0.77"));

    let (status, info) = route_request(
        build_service_router(Arc::clone(&state)),
        axum::http::Method::GET,
        &format!("/vms/{stopped_id}/info"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["private_address"], json!("10.128.0.77"));
    let (status, info) = route_request(
        build_service_router(Arc::clone(&state)),
        axum::http::Method::GET,
        "/vms/running-vm/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["private_address"], json!(running_address.to_string()));
}
