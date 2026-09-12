//! Named networks over the service API: created, listed, joined by VMs with
//! their lifetime addresses, and retired only once empty.
use super::*;
use axum::http::Method;

fn app(state: &Arc<ServiceState>) -> axum::Router {
    build_service_router(Arc::clone(state))
}

async fn create_network(state: &Arc<ServiceState>, name: &str) -> (StatusCode, serde_json::Value) {
    route_request(app(state), Method::POST, "/networks", Some(json!({ "name": name }))).await
}

#[tokio::test]
async fn networks_are_created_listed_inspected_and_retired() {
    let (state, _dir) = make_test_state_with_tempdir();
    let (status, created) = create_network(&state, "team").await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["name"], "team");
    assert_eq!(created["members"], json!([]));
    let id = created["id"].as_str().unwrap().to_string();

    let (status, duplicate) = create_network(&state, "team").await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");
    let (status, invalid) = create_network(&state, "Not A Label").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");

    let (status, list) = route_request(app(&state), Method::GET, "/networks", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["networks"].as_array().unwrap().len(), 1);
    assert_eq!(list["networks"][0]["id"], json!(id));

    let (status, inspected) = route_request(app(&state), Method::GET, &format!("/networks/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(inspected, created);
    let (status, _) = route_request(app(&state), Method::GET, "/networks/not-a-uuid", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = route_request(app(&state), Method::DELETE, &format!("/networks/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = route_request(app(&state), Method::GET, &format!("/networks/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "retired networks are history");
    let (status, again) = create_network(&state, "team").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(again["id"], json!(id), "a reused name is a new network");
}

#[tokio::test]
async fn members_join_with_their_lifetime_address_and_block_deletion() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "running-vm", 4242);
    let running_address = state.instances.lock().unwrap()["running-vm"].private_address;
    let stopped_dir = state.run_dir.join("persistent/stopped-vm");
    capsem_core::create_virtiofs_session(&stopped_dir, 64).unwrap();
    let mut stopped = test_persistent_entry("stopped-vm", stopped_dir);
    stopped.private_address = Some(std::net::Ipv4Addr::new(10, 128, 0, 77));
    let stopped_id = stopped.id.clone();
    state.persistent_registry.lock().unwrap().register(stopped).unwrap();
    let legacy_dir = state.run_dir.join("persistent/legacy-vm");
    capsem_core::create_virtiofs_session(&legacy_dir, 64).unwrap();
    let legacy = test_persistent_entry("legacy-vm", legacy_dir);
    let legacy_id = legacy.id.clone();
    state.persistent_registry.lock().unwrap().register(legacy).unwrap();

    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();
    let member = |vm: &str| format!("/networks/{id}/members/{vm}");

    let (status, joined) = route_request(app(&state), Method::PUT, &member("running-vm"), None).await;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["members"][0]["vm_id"], "running-vm");
    assert_eq!(joined["members"][0]["address"], json!(running_address.to_string()));
    assert_eq!(joined["members"][0]["state"], "declared");
    let (status, joined) = route_request(app(&state), Method::PUT, &member(&stopped_id), None).await;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(
        joined["members"].as_array().unwrap().len(),
        2,
        "a stopped VM joins with its recorded address"
    );

    let (status, refused) = route_request(app(&state), Method::PUT, &member(&legacy_id), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "no address yet: {refused}");
    let (status, _) = route_request(app(&state), Method::PUT, &member("ghost-vm"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, refused) = route_request(app(&state), Method::DELETE, &format!("/networks/{id}"), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");

    let (status, left) = route_request(app(&state), Method::DELETE, &member("running-vm"), None).await;
    assert_eq!(status, StatusCode::OK, "{left}");
    assert_eq!(left["members"].as_array().unwrap().len(), 1);
    let (status, _) = route_request(app(&state), Method::DELETE, &member("running-vm"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "leaving twice is not a member");
    let (status, _) = route_request(app(&state), Method::DELETE, &member(&stopped_id), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = route_request(app(&state), Method::DELETE, &format!("/networks/{id}"), None).await;
    assert_eq!(status, StatusCode::OK, "empty networks retire");
}

#[tokio::test]
async fn deleting_a_vm_leaves_every_network_it_was_in() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let stopped_dir = state.run_dir.join("persistent/stopped-vm");
    capsem_core::create_virtiofs_session(&stopped_dir, 64).unwrap();
    let mut stopped = test_persistent_entry("stopped-vm", stopped_dir);
    stopped.private_address = Some(std::net::Ipv4Addr::new(10, 128, 0, 77));
    let stopped_id = stopped.id.clone();
    state.persistent_registry.lock().unwrap().register(stopped).unwrap();
    let (_, a) = create_network(&state, "alpha").await;
    let (_, b) = create_network(&state, "beta").await;
    for network in [&a, &b] {
        let path = format!("/networks/{}/members/{stopped_id}", network["id"].as_str().unwrap());
        let (status, _) = route_request(app(&state), Method::PUT, &path, None).await;
        assert_eq!(status, StatusCode::OK);
    }

    let (status, deleted) =
        route_request(app(&state), Method::DELETE, &format!("/vms/{stopped_id}/delete"), None).await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    for network in [&a, &b] {
        let id = network["id"].as_str().unwrap();
        let (status, _) = route_request(app(&state), Method::GET, &format!("/networks/{id}"), None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "the last member took the network with it"
        );
        let (status, history) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
        assert_eq!(status, StatusCode::OK, "its history stays readable: {history}");
        let (status, _) = route_request(
            app(&state),
            Method::PUT,
            &format!("/networks/{id}/members/{stopped_id}"),
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "nothing resurrects a retired network or a deleted VM"
        );
    }
    let (status, list) = route_request(app(&state), Method::GET, "/networks", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["networks"], json!([]));
}

#[tokio::test]
async fn stopping_a_vm_keeps_its_membership_and_a_fork_has_none() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("sessions/fork-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    insert_fake_instance_with_session_dir(&state, "fork-src", std::process::id(), session_dir);
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();
    let (status, _) = route_request(
        app(&state),
        Method::PUT,
        &format!("/networks/{id}/members/fork-src"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let fork = handle_fork(
        State(Arc::clone(&state)),
        Path("fork-src".into()),
        Json(ForkRequest {
            name: "my-fork".into(),
            description: None,
        }),
    )
    .await
    .unwrap();
    let (_, inspected) = route_request(app(&state), Method::GET, &format!("/networks/{id}"), None).await;
    let members: Vec<&str> = inspected["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| member["vm_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        members,
        vec!["fork-src"],
        "a fork is a new VM with no membership: {inspected}"
    );
    assert_ne!(fork.0.id, "fork-src");

    // Stopping evicts the instance; the membership is the VM's, not the run's.
    assert!(state.evict_instance("fork-src").is_some());
    let (_, inspected) = route_request(app(&state), Method::GET, &format!("/networks/{id}"), None).await;
    assert_eq!(inspected["members"][0]["vm_id"], "fork-src", "{inspected}");
}

#[tokio::test]
async fn network_logs_page_with_a_cursor_and_refuse_a_foreign_one() {
    use capsem_logger::{TransportEvent, TransportEventKind, WriteOp};
    let (state, _dir) = make_test_state_with_tempdir();
    let (_, created) = create_network(&state, "audited").await;
    let id = created["id"].as_str().unwrap().to_string();
    let network = uuid::Uuid::parse_str(&id).unwrap();
    let handle = state.networks.lock().await.reader(network).unwrap();
    for n in 0..3u8 {
        let event = TransportEvent::new(
            format!("{:012x}", 0xfeed00 + u32::from(n)),
            1_000 + i64::from(n),
            TransportEventKind::Connect,
            Some(network),
            Some(uuid::Uuid::from_u128(1 + u128::from(n))),
            &json!({ "decision": { "effective": "allow" } }),
        )
        .unwrap();
        handle.write(WriteOp::TransportEvent(event)).await.unwrap();
    }
    handle.flush().await.unwrap();
    let (status, page) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs?limit=2"), None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["events"].as_array().unwrap().len(), 2);
    assert_eq!(page["events"][0]["event"]["decision"]["effective"], "allow");
    let next = page["next_cursor"].as_str().expect("a full page continues").to_string();
    let (status, rest) = route_request(
        app(&state),
        Method::GET,
        &format!("/networks/{id}/logs?limit=2&cursor={next}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rest}");
    assert_eq!(rest["events"].as_array().unwrap().len(), 1);
    assert!(rest["next_cursor"].is_null());
    let (status, refused) = route_request(
        app(&state),
        Method::GET,
        &format!("/networks/{id}/logs?limit=2&cursor={next}&type=network.close"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, _) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs?limit=0"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
