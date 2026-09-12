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

fn owner_secret(state: &ServiceState, vm: &str, secret: &str) {
    state.instances.lock().unwrap().get_mut(vm).unwrap().owner_secret = secret.into();
}

async fn private_connect(state: &Arc<ServiceState>, request: serde_json::Value) -> (StatusCode, serde_json::Value) {
    route_request(app(state), Method::POST, "/networks/private/connect", Some(request)).await
}

async fn private_datagram(state: &Arc<ServiceState>, request: serde_json::Value) -> (StatusCode, serde_json::Value) {
    route_request(app(state), Method::POST, "/networks/private/datagram", Some(request)).await
}

#[tokio::test]
async fn a_private_datagram_flow_is_admitted_like_a_connection_and_names_its_protocol() {
    let (state, dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-a", 4242);
    insert_fake_instance(&state, "vm-b", 4243);
    owner_secret(&state, "vm-a", "secret-a");
    let address_b = state.instances.lock().unwrap()["vm-b"].private_address;
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();
    for vm in ["vm-a", "vm-b"] {
        let (status, _) = route_request(app(&state), Method::PUT, &format!("/networks/{id}/members/{vm}"), None).await;
        assert_eq!(status, StatusCode::OK);
    }
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    let owner_b = spawn_fake_process(&uds_b, 2, |message| {
        let reply = match message {
            ServiceToProcess::PrivateAccept {
                id,
                protocol,
                port,
                source_port,
                ..
            } => {
                match protocol.as_str() {
                    "udp" => assert_eq!((*port, *source_port), (5353, 40000)),
                    "icmp" => assert_eq!((*port, *source_port), (0, 0x4242)),
                    other => panic!("unexpected protocol {other}"),
                }
                Some(ProcessToService::PrivateAcceptResult {
                    id: *id,
                    handoff_socket: "/run/vm-b/private-relay.sock".into(),
                    error: None,
                })
            }
            other => panic!("unexpected owner message: {other:?}"),
        };
        Box::pin(async move { reply })
    });
    let base = json!({ "source_vm": "vm-a", "owner_secret": "secret-a", "destination": address_b.to_string() });
    let mut udp = base.clone();
    udp["protocol"] = json!("udp");
    udp["port"] = json!(5353);
    udp["source_port"] = json!(40000);
    let (status, admitted) = private_datagram(&state, udp).await;
    assert_eq!(status, StatusCode::OK, "{admitted}");
    assert_eq!(admitted["relay_socket"], "/run/vm-b/private-relay.sock");
    assert_eq!(admitted["destination_vm"], "vm-b");
    assert_eq!(admitted["token"].as_str().unwrap().len(), 16);
    let mut icmp = base.clone();
    icmp["protocol"] = json!("icmp");
    icmp["port"] = json!(0);
    icmp["source_port"] = json!(0x4242);
    let (status, admitted) = private_datagram(&state, icmp).await;
    assert_eq!(status, StatusCode::OK, "{admitted}");
    assert_eq!(owner_b.await.unwrap().len(), 2);

    // What the service refuses before any owner is asked.
    let mut portless_udp = base.clone();
    portless_udp["protocol"] = json!("udp");
    portless_udp["port"] = json!(0);
    portless_udp["source_port"] = json!(40000);
    assert_eq!(private_datagram(&state, portless_udp).await.0, StatusCode::BAD_REQUEST);
    let mut icmp_with_port = base.clone();
    icmp_with_port["protocol"] = json!("icmp");
    icmp_with_port["port"] = json!(7);
    icmp_with_port["source_port"] = json!(1);
    assert_eq!(
        private_datagram(&state, icmp_with_port).await.0,
        StatusCode::BAD_REQUEST
    );
    let mut sctp = base.clone();
    sctp["protocol"] = json!("sctp");
    sctp["port"] = json!(7);
    sctp["source_port"] = json!(1);
    assert_eq!(private_datagram(&state, sctp).await.0, StatusCode::BAD_REQUEST);
    let mut stranger = base.clone();
    stranger["protocol"] = json!("udp");
    stranger["destination"] = json!("10.128.0.9");
    stranger["port"] = json!(53);
    stranger["source_port"] = json!(40000);
    assert_eq!(private_datagram(&state, stranger).await.0, StatusCode::NOT_FOUND);

    let (status, logs) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
    assert_eq!(status, StatusCode::OK, "{logs}");
    let events = logs["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "one row per admitted flow: {logs}");
    let protocols: Vec<_> = events
        .iter()
        .map(|event| event["event"]["network"]["protocol"].as_str().unwrap().to_string())
        .collect();
    assert!(
        protocols.contains(&"udp".to_string()) && protocols.contains(&"icmp".to_string()),
        "{protocols:?}"
    );
    assert!(
        events
            .iter()
            .all(|event| event["event"]["decision"]["effective"] == "allow"),
        "{logs}"
    );
    drop(dir);
}

#[tokio::test]
async fn a_private_connection_is_admitted_through_the_destination_owner_and_audited_for_both() {
    let (state, dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-a", 4242);
    insert_fake_instance(&state, "vm-b", 4243);
    owner_secret(&state, "vm-a", "secret-a");
    let address_b = state.instances.lock().unwrap()["vm-b"].private_address;
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();
    for vm in ["vm-a", "vm-b"] {
        let (status, _) = route_request(app(&state), Method::PUT, &format!("/networks/{id}/members/{vm}"), None).await;
        assert_eq!(status, StatusCode::OK);
    }
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    let owner_b = spawn_fake_process(&uds_b, 1, |message| {
        let reply = match message {
            ServiceToProcess::PrivateAccept {
                id,
                token,
                source_vm,
                port,
                ..
            } => {
                assert_eq!(source_vm, "vm-a");
                assert_eq!(*port, 6379);
                assert!(!token.is_empty());
                Some(ProcessToService::PrivateAcceptResult {
                    id: *id,
                    handoff_socket: "/run/vm-b/private-handoff.sock".into(),
                    error: None,
                })
            }
            other => panic!("unexpected owner message: {other:?}"),
        };
        Box::pin(async move { reply })
    });
    let request = json!({
        "source_vm": "vm-a", "owner_secret": "secret-a",
        "destination": address_b.to_string(), "port": 6379, "source_port": 40001, "process_name": "redis-cli",
    });

    let (status, admitted) = private_connect(&state, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{admitted}");
    assert_eq!(admitted["destination_vm"], "vm-b");
    assert_eq!(admitted["network"], json!(id));
    assert_eq!(admitted["handoff_socket"], "/run/vm-b/private-handoff.sock");
    assert!(!admitted["token"].as_str().unwrap().is_empty());
    let messages = owner_b.await.unwrap();
    assert!(matches!(messages[0], ServiceToProcess::PrivateAccept { .. }));

    let (status, logs) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
    assert_eq!(status, StatusCode::OK, "{logs}");
    let events = logs["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{logs}");
    let facts = &events[0]["event"];
    assert_eq!(facts["network"]["context"], "private");
    assert_eq!(facts["network"]["source"]["vm"]["id"], "vm-a");
    assert_eq!(facts["network"]["destination"]["vm"]["id"], "vm-b");
    assert_eq!(facts["network"]["destination"]["port"], 6379);
    assert_eq!(facts["decision"]["effective"], "allow");
    let (_, by_a) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs?vm=vm-a"), None).await;
    let (_, by_b) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs?vm=vm-b"), None).await;
    assert_eq!(
        by_a["events"].as_array().unwrap().len(),
        1,
        "visible from the source: {by_a}"
    );
    assert_eq!(
        by_b["events"].as_array().unwrap().len(),
        1,
        "and from the destination: {by_b}"
    );
    drop(dir);
}

#[tokio::test]
async fn a_private_connection_is_refused_before_any_owner_is_asked() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-a", 4242);
    insert_fake_instance(&state, "vm-b", 4243);
    insert_fake_instance(&state, "vm-c", 4244);
    owner_secret(&state, "vm-a", "secret-a");
    let address_b = state.instances.lock().unwrap()["vm-b"].private_address.to_string();
    let address_c = state.instances.lock().unwrap()["vm-c"].private_address.to_string();
    let (_, team) = create_network(&state, "team").await;
    let (_, other) = create_network(&state, "other").await;
    for (network, vm) in [(&team, "vm-a"), (&team, "vm-b"), (&other, "vm-c")] {
        let path = format!("/networks/{}/members/{vm}", network["id"].as_str().unwrap());
        let (status, _) = route_request(app(&state), Method::PUT, &path, None).await;
        assert_eq!(status, StatusCode::OK);
    }
    let request = |secret: &str, destination: &str| json!({ "source_vm": "vm-a", "owner_secret": secret, "destination": destination, "port": 80, "source_port": 40001 });
    // No fake owner listens anywhere: every refusal below happens first.
    let (status, _) = private_connect(&state, request("wrong", &address_b)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a wrong secret is not an owner");
    let (status, _) = private_connect(&state, request("secret-a", &address_c)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a member of another network is unreachable"
    );
    let (status, _) = private_connect(&state, request("secret-a", "10.128.7.7")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "nobody's address");
    let (status, _) = private_connect(
        &state,
        json!({ "source_vm": "ghost", "owner_secret": "x", "destination": address_b, "port": 80, "source_port": 40001 }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no such running VM");

    // The destination is a member but stopped: refused and audited as such.
    assert!(state.evict_instance("vm-b").is_some());
    let (status, refused) = private_connect(&state, request("secret-a", &address_b)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    let id = team["id"].as_str().unwrap();
    let (_, logs) = route_request(
        app(&state),
        Method::GET,
        &format!("/networks/{id}/logs?decision=block"),
        None,
    )
    .await;
    let events = logs["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{logs}");
    assert_eq!(events[0]["event"]["decision"]["reason"], "destination_stopped");
}

#[tokio::test]
async fn a_destination_owner_that_refuses_or_never_answers_blocks_and_is_audited() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-a", 4242);
    insert_fake_instance(&state, "vm-b", 4243);
    owner_secret(&state, "vm-a", "secret-a");
    let address_b = state.instances.lock().unwrap()["vm-b"].private_address.to_string();
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    let (_, team) = create_network(&state, "team").await;
    let id = team["id"].as_str().unwrap().to_string();
    for vm in ["vm-a", "vm-b"] {
        let (status, _) = route_request(app(&state), Method::PUT, &format!("/networks/{id}/members/{vm}"), None).await;
        assert_eq!(status, StatusCode::OK);
    }
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    let owner_b = spawn_fake_process(&uds_b, 1, |message| {
        let reply = match message {
            ServiceToProcess::PrivateAccept { id, .. } => Some(ProcessToService::PrivateAcceptResult {
                id: *id,
                handoff_socket: String::new(),
                error: Some("private handoff not available on this owner".into()),
            }),
            other => panic!("unexpected owner message: {other:?}"),
        };
        Box::pin(async move { reply })
    });
    let request = json!({ "source_vm": "vm-a", "owner_secret": "secret-a", "destination": address_b, "port": 80, "source_port": 40001 });
    let (status, refused) = private_connect(&state, request.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    owner_b.await.unwrap();
    let (_, logs) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
    let events = logs["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{logs}");
    assert_eq!(events[0]["event"]["decision"]["reason"], "destination_refused");

    // Nobody listening on the destination's socket: unreachable, not a hang.
    let _ = std::fs::remove_file(&uds_b);
    let (status, refused) = private_connect(&state, request).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{refused}");
    let (_, logs) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
    assert_eq!(logs["events"].as_array().unwrap().len(), 2, "{logs}");
    assert_eq!(
        logs["events"][1]["event"]["decision"]["reason"],
        "destination_unreachable"
    );
}

#[tokio::test]
async fn a_membership_that_changes_while_the_owner_answers_is_never_granted() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-a", 4242);
    insert_fake_instance(&state, "vm-b", 4243);
    owner_secret(&state, "vm-a", "secret-a");
    let address_b = state.instances.lock().unwrap()["vm-b"].private_address.to_string();
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    let (_, team) = create_network(&state, "team").await;
    let id = team["id"].as_str().unwrap().to_string();
    for vm in ["vm-a", "vm-b"] {
        let (status, _) = route_request(app(&state), Method::PUT, &format!("/networks/{id}/members/{vm}"), None).await;
        assert_eq!(status, StatusCode::OK);
    }
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    // The destination owner says yes, but by then vm-b has left the network.
    let detaching = Arc::clone(&state);
    let network = uuid::Uuid::parse_str(&id).unwrap();
    let owner_b = spawn_fake_process(&uds_b, 1, move |message| {
        let state = Arc::clone(&detaching);
        let reply = match message {
            ServiceToProcess::PrivateAccept { id, .. } => Some(ProcessToService::PrivateAcceptResult {
                id: *id,
                handoff_socket: "/run/vm-b/private-handoff.sock".into(),
                error: None,
            }),
            other => panic!("unexpected owner message: {other:?}"),
        };
        Box::pin(async move {
            state.networks.lock().await.detach(network, "vm-b", 9).await.unwrap();
            reply
        })
    });
    let request = json!({ "source_vm": "vm-a", "owner_secret": "secret-a", "destination": address_b, "port": 80, "source_port": 40001 });
    let (status, refused) = private_connect(&state, request).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    owner_b.await.unwrap();
    let (_, logs) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
    let events = logs["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{logs}");
    assert_eq!(events[0]["event"]["decision"]["effective"], "block");
    assert_eq!(events[0]["event"]["decision"]["reason"], "membership_changed");
}
