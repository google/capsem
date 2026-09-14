//! Named networks over the service API: created with a subnet, listed, joined
//! by VMs that lease an address in it, and retired only once empty.
use super::*;
use axum::http::Method;
use std::time::Duration;

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
async fn members_lease_an_address_in_the_networks_subnet_and_block_deletion() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "running-vm", 4242);
    let stopped_dir = state.run_dir.join("persistent/stopped-vm");
    capsem_core::create_virtiofs_session(&stopped_dir, 64).unwrap();
    let stopped = test_persistent_entry("stopped-vm", stopped_dir);
    let stopped_id = stopped.id.clone();
    state.persistent_registry.lock().unwrap().register(stopped).unwrap();

    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();
    let subnet = capsem_config::PrivatePool::parse(created["subnet"].as_str().unwrap()).unwrap();
    let member = |network: &str, vm: &str| format!("/networks/{network}/members/{vm}");

    let (status, joined) = route_request(app(&state), Method::PUT, &member(&id, "running-vm"), None).await;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["members"][0]["vm_id"], "running-vm");
    let running_address: std::net::Ipv4Addr = joined["members"][0]["address"].as_str().unwrap().parse().unwrap();
    assert!(
        subnet.contains(running_address),
        "{running_address} is outside {subnet}"
    );
    // A running VM is plugged at once; this fake owner has no seat, so the
    // plug fails and the membership says so.
    assert_eq!(joined["members"][0]["state"], "failed");
    let (status, joined) = route_request(app(&state), Method::PUT, &member(&id, &stopped_id), None).await;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(
        joined["members"].as_array().unwrap().len(),
        2,
        "a stopped VM joins too, and is plugged when it runs"
    );
    let (status, again) = route_request(app(&state), Method::PUT, &member(&id, "running-vm"), None).await;
    assert_eq!(status, StatusCode::OK);
    let running = again["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["vm_id"] == "running-vm")
        .unwrap();
    assert_eq!(
        running["address"],
        json!(running_address.to_string()),
        "joining again keeps the lease"
    );
    let (status, _) = route_request(app(&state), Method::PUT, &member(&id, "ghost-vm"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, other) = create_network(&state, "other").await;
    let other_id = other["id"].as_str().unwrap().to_string();
    let other_subnet = capsem_config::PrivatePool::parse(other["subnet"].as_str().unwrap()).unwrap();
    assert!(!subnet.overlaps(other_subnet), "{subnet} and {other_subnet}");
    let (status, joined_other) = route_request(app(&state), Method::PUT, &member(&other_id, "running-vm"), None).await;
    assert_eq!(status, StatusCode::OK, "{joined_other}");
    let other_address: std::net::Ipv4Addr = joined_other["members"][0]["address"].as_str().unwrap().parse().unwrap();
    assert!(
        other_subnet.contains(other_address),
        "a VM on two networks has an address in each"
    );

    let (status, refused) = route_request(app(&state), Method::DELETE, &format!("/networks/{id}"), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");

    let (status, left) = route_request(app(&state), Method::DELETE, &member(&id, "running-vm"), None).await;
    assert_eq!(status, StatusCode::OK, "{left}");
    assert_eq!(left["members"].as_array().unwrap().len(), 1);
    let (status, _) = route_request(app(&state), Method::DELETE, &member(&id, "running-vm"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "leaving twice is not a member");
    let (status, _) = route_request(app(&state), Method::DELETE, &member(&id, &stopped_id), None).await;
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
    let stopped = test_persistent_entry("stopped-vm", stopped_dir);
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

/// The TCP admission rows of a logs page; a running member's link rows
/// (these fake owners have no seat, so theirs say `block`) sit beside them.
fn tcp_events(logs: &serde_json::Value) -> Vec<serde_json::Value> {
    logs["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["event"]["network"]["protocol"] == "tcp")
        .cloned()
        .collect()
}

async fn private_connect(state: &Arc<ServiceState>, request: serde_json::Value) -> (StatusCode, serde_json::Value) {
    route_request(app(state), Method::POST, "/networks/private/connect", Some(request)).await
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
    let events = tcp_events(&logs);
    assert_eq!(events.len(), 1, "{logs}");
    let facts = &events[0]["event"];
    assert_eq!(facts["network"]["context"], "private");
    assert_eq!(facts["network"]["source"]["vm"]["id"], "vm-a");
    assert_eq!(facts["network"]["destination"]["vm"]["id"], "vm-b");
    assert_eq!(facts["network"]["destination"]["port"], 6379);
    assert_eq!(facts["decision"]["effective"], "allow");
    let (_, by_a) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs?vm=vm-a"), None).await;
    let (_, by_b) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs?vm=vm-b"), None).await;
    assert_eq!(tcp_events(&by_a).len(), 1, "visible from the source: {by_a}");
    assert_eq!(tcp_events(&by_b).len(), 1, "and from the destination: {by_b}");
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
    let events = tcp_events(&logs);
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
    let events = tcp_events(&logs);
    assert_eq!(events.len(), 1, "{logs}");
    assert_eq!(events[0]["event"]["decision"]["reason"], "destination_refused");

    // Nobody listening on the destination's socket: unreachable, not a hang.
    let _ = std::fs::remove_file(&uds_b);
    let (status, refused) = private_connect(&state, request).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{refused}");
    let (_, logs) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
    let events = tcp_events(&logs);
    assert_eq!(events.len(), 2, "{logs}");
    assert_eq!(events[1]["event"]["decision"]["reason"], "destination_unreachable");
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
    let events = tcp_events(&logs);
    assert_eq!(events.len(), 1, "{logs}");
    assert_eq!(events[0]["event"]["decision"]["effective"], "block");
    assert_eq!(events[0]["event"]["decision"]["reason"], "membership_changed");
}

/// A fake owner's link seat: answers LinkAttach with its handoff socket,
/// takes the token there `links` times, hands back one end of a fresh
/// socket pair as the guest stream each time, and reports each time the
/// service lets a link go.
struct FakeLinkSeat {
    /// The guest end of each stream handed over, in order.
    guest_ends: Mutex<Vec<std::os::unix::net::UnixStream>>,
    /// One signal per handoff connection the service let go of.
    released: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<()>>>,
    /// The owner answers a LinkAttach only with a permit: a test takes the
    /// permits away to hold a link mid-handshake.
    answers: Arc<tokio::sync::Semaphore>,
}

impl FakeLinkSeat {
    fn guest_end(&self) -> std::os::unix::net::UnixStream {
        self.guest_ends.lock().unwrap().remove(0)
    }
}

/// `detaches` is how many LinkDetach requests the owner answers besides
/// its `links` LinkAttach ones.
fn fake_link_seat(
    uds: &std::path::Path,
    refuse: bool,
    links: usize,
    detaches: usize,
) -> (Arc<FakeLinkSeat>, tokio::task::JoinHandle<Vec<ServiceToProcess>>) {
    use capsem_foundation::unix::router_channel::{Receiver, Sender};
    use std::os::fd::AsRawFd;
    let handoff = uds.with_file_name("vm-b-handoff.sock");
    let _ = std::fs::remove_file(&handoff);
    let listener = tokio::net::UnixListener::bind(&handoff).unwrap();
    let (released_tx, released_rx) = tokio::sync::mpsc::unbounded_channel();
    let seat = Arc::new(FakeLinkSeat {
        guest_ends: Mutex::new(Vec::new()),
        released: Mutex::new(Some(released_rx)),
        answers: Arc::new(tokio::sync::Semaphore::new(links)),
    });
    let answers = Arc::clone(&seat.answers);
    let ends = Arc::clone(&seat);
    tokio::spawn(async move {
        for _ in 0..links {
            let (switch_end, guest_end) = std::os::unix::net::UnixStream::pair().unwrap();
            ends.guest_ends.lock().unwrap().push(guest_end);
            let (stream, _) = listener.accept().await.unwrap();
            let std = stream.into_std().unwrap();
            let receiver = Receiver::new(std.try_clone().unwrap()).unwrap();
            let frame = receiver.recv().await.unwrap();
            assert_eq!(frame.bytes[1], 5, "a link request");
            let sender = Sender::new(std.try_clone().unwrap()).unwrap();
            sender.send(&frame.bytes, &[switch_end.as_raw_fd()]).await.unwrap();
            std.set_nonblocking(true).unwrap();
            let released = released_tx.clone();
            tokio::spawn(async move {
                // Darwin may flush a descriptor only an in-flight message
                // refers to: the owner keeps its copy until the service lets
                // go, as the real seat does.
                let _switch_end = switch_end;
                let mut watch = tokio::net::UnixStream::from_std(std).unwrap();
                let mut sink = [0u8; 8];
                while tokio::io::AsyncReadExt::read(&mut watch, &mut sink).await.unwrap_or(0) != 0 {}
                let _ = released.send(());
            });
        }
    });
    let owner = spawn_fake_process(uds, links + detaches, move |message| {
        if let ServiceToProcess::LinkDetach { id, .. } = message {
            let reply = ProcessToService::LinkDetachResult { id: *id, error: None };
            return Box::pin(async move { Some(reply) });
        }
        let reply = match message {
            ServiceToProcess::LinkAttach {
                id,
                token,
                network_name,
                ..
            } => {
                assert_eq!(token.len(), 16);
                assert_eq!(network_name, "team");
                Some(ProcessToService::LinkAttachResult {
                    id: *id,
                    handoff_socket: if refuse {
                        String::new()
                    } else {
                        handoff.to_string_lossy().into_owned()
                    },
                    error: refuse.then(|| "this VM's profile blocks the link".into()),
                })
            }
            other => panic!("unexpected owner message: {other:?}"),
        };
        let answers = Arc::clone(&answers);
        Box::pin(async move {
            answers.acquire().await.unwrap().forget();
            reply
        })
    });
    (seat, owner)
}

async fn member_state(state: &Arc<ServiceState>, network: &str, vm: &str) -> String {
    let (_, info) = route_request(app(state), Method::GET, &format!("/networks/{network}"), None).await;
    info["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["vm_id"] == vm)
        .map(|member| member["state"].as_str().unwrap().to_string())
        .unwrap_or_else(|| "absent".into())
}

#[tokio::test]
async fn attaching_a_running_member_links_it_to_the_networks_switch_until_it_leaves() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-b", std::process::id());
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    let (seat, owner) = fake_link_seat(&uds_b, false, 1, 1);
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();

    let (status, joined) = route_request(app(&state), Method::PUT, &format!("/networks/{id}/members/vm-b"), None).await;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["members"][0]["state"], "ready", "{joined}");
    let lease: std::net::Ipv4Addr = joined["members"][0]["address"].as_str().unwrap().parse().unwrap();
    let (_, logs) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
    let linked = logs["events"].as_array().unwrap().iter().any(|event| {
        event["event"]["network"]["protocol"] == "link"
            && event["event"]["network"]["destination"]["vm"]["id"] == "vm-b"
            && event["event"]["decision"]["effective"] == "allow"
    });
    assert!(linked, "{logs}");

    // Leaving unlinks: the switch drops the stream and the owner's handoff
    // connection ends, so it lets go of the guest stream.
    let (status, _) = route_request(
        app(&state),
        Method::DELETE,
        &format!("/networks/{id}/members/vm-b"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut released = seat.released.lock().unwrap().take().unwrap();
    tokio::time::timeout(Duration::from_secs(3), released.recv())
        .await
        .expect("the service let the link go")
        .unwrap();
    let guest_end = seat.guest_end();
    guest_end.set_nonblocking(true).unwrap();
    let mut guest_end = tokio::net::UnixStream::from_std(guest_end).unwrap();
    let read = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::io::AsyncReadExt::read(&mut guest_end, &mut [0u8; 1]),
    )
    .await
    .expect("the switch closed the stream")
    .unwrap();
    assert_eq!(read, 0);
    assert_eq!(member_state(&state, &id, "vm-b").await, "absent");

    // The owner was told the member's address to bring the cable up with,
    // and to take the cable down once the VM left.
    let messages = tokio::time::timeout(Duration::from_secs(5), owner)
        .await
        .expect("the owner hears the plug and the detach")
        .unwrap();
    match &messages[..] {
        [ServiceToProcess::LinkAttach {
            network,
            address,
            prefix,
            ..
        }, ServiceToProcess::LinkDetach { network: detached, .. }] => {
            assert_eq!((network, *address, *prefix), (&id, lease, 24));
            assert_eq!(detached, &id);
        }
        other => panic!("unexpected owner messages {other:?}"),
    }
}

/// A freshly spawned owner binds its socket some time after the service
/// registers it. Linking at start tried once, found no socket, and left the
/// member unlinked for good: its admitted TCP worked and its UDP never did.
#[tokio::test]
async fn a_member_links_at_start_even_when_its_owner_binds_late() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-b", std::process::id());
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();

    // As provisioning does: record the membership, then link it.
    crate::network_routes::attach_provisioned(&state, "vm-b", &[id.parse().unwrap()])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (_seat, owner) = fake_link_seat(&uds_b, false, 1, 0);

    let mut observed = String::new();
    for _ in 0..100 {
        observed = member_state(&state, &id, "vm-b").await;
        if observed == "ready" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(observed, "ready", "the member was never linked");
    assert!(matches!(owner.await.unwrap()[0], ServiceToProcess::LinkAttach { .. }));
}

#[tokio::test]
async fn a_member_whose_stream_ends_is_declared_and_then_linked_again() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-b", std::process::id());
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    let (seat, owner) = fake_link_seat(&uds_b, false, 2, 0);
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();
    let (status, _) = route_request(app(&state), Method::PUT, &format!("/networks/{id}/members/vm-b"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(member_state(&state, &id, "vm-b").await, "ready");
    // The guest's end goes away (its pump died): the switch reports the
    // close, the membership falls back to declared, the owner is let go, and
    // the running member is asked for its fresh stream and is ready again.
    drop(seat.guest_end());
    let mut released = seat.released.lock().unwrap().take().unwrap();
    tokio::time::timeout(Duration::from_secs(3), released.recv())
        .await
        .expect("the service let the link go")
        .unwrap();
    let mut seen = Vec::new();
    for _ in 0..200 {
        let observed = member_state(&state, &id, "vm-b").await;
        if seen.last() != Some(&observed) {
            seen.push(observed.clone());
        }
        if seen.contains(&"declared".to_string()) && observed == "ready" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (_, history) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
    assert!(
        seen.contains(&"declared".to_string()) && seen.last() == Some(&"ready".to_string()),
        "{seen:?} {history}"
    );
    let messages = owner.await.unwrap();
    assert_eq!(messages.len(), 2, "asked twice: once on attach, once after the close");
    let (_, logs) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
    let closed = logs["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "network.close" && event["event"]["network"]["protocol"] == "link");
    assert!(closed, "{logs}");
}

async fn released_within(seat: &FakeLinkSeat, wait: Duration) -> bool {
    let mut released = seat.released.lock().unwrap().take().unwrap();
    tokio::time::timeout(wait, released.recv()).await.is_ok()
}

#[tokio::test]
async fn a_member_that_leaves_before_its_relink_is_not_linked_back_in() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-b", std::process::id());
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    // Room for a second plug that must never come, besides the plug and the
    // detach that do.
    let (seat, owner) = fake_link_seat(&uds_b, false, 2, 1);
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();
    let (status, _) = route_request(app(&state), Method::PUT, &format!("/networks/{id}/members/vm-b"), None).await;
    assert_eq!(status, StatusCode::OK);
    // The pump dies, and the VM is disconnected inside the relink window.
    drop(seat.guest_end());
    assert!(
        released_within(&seat, Duration::from_secs(3)).await,
        "the service let the link go"
    );
    let (status, _) = route_request(
        app(&state),
        Method::DELETE,
        &format!("/networks/{id}/members/vm-b"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(member_state(&state, &id, "vm-b").await, "absent");
    assert!(
        !owner.is_finished(),
        "a member that left is never asked for its link again"
    );
}

#[tokio::test]
async fn a_member_that_leaves_mid_handshake_keeps_no_link() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-b", std::process::id());
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    let (seat, _owner) = fake_link_seat(&uds_b, false, 1, 0);
    seat.answers.forget_permits(1);
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();
    let joining = {
        let (router, uri) = (app(&state), format!("/networks/{id}/members/vm-b"));
        tokio::spawn(async move { route_request(router, Method::PUT, &uri, None).await })
    };
    for _ in 0..100 {
        if member_state(&state, &id, "vm-b").await == "attaching" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(member_state(&state, &id, "vm-b").await, "attaching");
    let (status, _) = route_request(
        app(&state),
        Method::DELETE,
        &format!("/networks/{id}/members/vm-b"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    seat.answers.add_permits(1);
    let (status, _) = joining.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(member_state(&state, &id, "vm-b").await, "absent");
    assert!(
        released_within(&seat, Duration::from_secs(3)).await,
        "the link granted to a member that already left is let go"
    );
}

/// Finding 1 of the PR #200 review. Disconnect used to unplug first and
/// revoke the membership second: a plug that finished between the two saw a
/// member, kept its port, and the disconnect then returned success with the
/// VM still on the network. The window is held open here, and the plug is
/// finished inside it.
#[tokio::test]
async fn a_plug_that_finishes_inside_a_disconnect_leaves_no_port_behind() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-b", std::process::id());
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    let (seat, _owner) = fake_link_seat(&uds_b, false, 1, 0);
    seat.answers.forget_permits(1);
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();
    let network: uuid::Uuid = id.parse().unwrap();
    let joining = {
        let (router, uri) = (app(&state), format!("/networks/{id}/members/vm-b"));
        tokio::spawn(async move { route_request(router, Method::PUT, &uri, None).await })
    };
    for _ in 0..100 {
        if member_state(&state, &id, "vm-b").await == "attaching" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(member_state(&state, &id, "vm-b").await, "attaching");

    let window = Arc::new((tokio::sync::Notify::new(), tokio::sync::Notify::new()));
    *state.switches.detach_window.lock().unwrap() = Some(Arc::clone(&window));
    let leaving = {
        let (router, uri) = (app(&state), format!("/networks/{id}/members/vm-b"));
        tokio::spawn(async move { route_request(router, Method::DELETE, &uri, None).await })
    };
    tokio::time::timeout(Duration::from_secs(5), window.0.notified())
        .await
        .expect("the disconnect reached its window");
    // The owner answers now: the plug gets its port inside the window.
    seat.answers.add_permits(1);
    let (status, _) = joining.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    window.1.notify_one();
    let (status, _) = leaving.await.unwrap();
    assert_eq!(status, StatusCode::OK);

    assert_eq!(member_state(&state, &id, "vm-b").await, "absent");
    assert!(
        state.switches.plugged(network).await.is_empty(),
        "a disconnected member kept its port"
    );
    assert!(
        released_within(&seat, Duration::from_secs(3)).await,
        "the owner's stream was never let go"
    );
    let guest_end = seat.guest_end();
    guest_end.set_nonblocking(true).unwrap();
    let mut guest_end = tokio::net::UnixStream::from_std(guest_end).unwrap();
    let read = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::io::AsyncReadExt::read(&mut guest_end, &mut [0u8; 1]),
    )
    .await
    .expect("the switch closed the disconnected member's cable")
    .unwrap();
    assert_eq!(read, 0);
}

#[tokio::test]
async fn an_owner_that_refuses_the_link_leaves_a_failed_membership() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    insert_fake_instance(&state, "vm-b", std::process::id());
    let uds_b = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    std::fs::create_dir_all(uds_b.parent().unwrap()).unwrap();
    let (_seat, _owner) = fake_link_seat(&uds_b, true, 1, 0);
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap().to_string();
    let (status, joined) = route_request(app(&state), Method::PUT, &format!("/networks/{id}/members/vm-b"), None).await;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["members"][0]["state"], "failed", "{joined}");
    let (_, logs) = route_request(app(&state), Method::GET, &format!("/networks/{id}/logs"), None).await;
    let blocked = logs["events"].as_array().unwrap().iter().any(|event| {
        event["event"]["network"]["protocol"] == "link"
            && event["event"]["decision"]["effective"] == "block"
            && event["event"]["decision"]["reason"]
                .as_str()
                .unwrap()
                .contains("blocks the link")
    });
    assert!(blocked, "{logs}");
}

async fn private_resolve(state: &Arc<ServiceState>, request: serde_json::Value) -> (StatusCode, serde_json::Value) {
    route_request(app(state), Method::POST, "/networks/private/resolve", Some(request)).await
}

#[tokio::test]
async fn private_names_resolve_only_to_members_the_asker_shares_a_network_with() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    for (id, name) in [
        ("vm-a", "alpha"),
        ("vm-b", "beta"),
        ("vm-c", "gamma"),
        ("vm-d", "beta"),
        ("vm-e", "echo"),
    ] {
        insert_fake_instance(&state, id, std::process::id());
        state.instances.lock().unwrap().get_mut(id).unwrap().name = name.into();
    }
    owner_secret(&state, "vm-a", "secret-a");
    // Each VM but vm-a is in one network, so its lease there is its address.
    let mut addresses: HashMap<&str, String> = HashMap::new();
    let (_, team) = create_network(&state, "team").await;
    let (_, other) = create_network(&state, "other").await;
    let (_, apart) = create_network(&state, "apart").await;
    let team = team["id"].as_str().unwrap().to_string();
    let other = other["id"].as_str().unwrap().to_string();
    let apart = apart["id"].as_str().unwrap().to_string();
    for (network, vm) in [
        (&team, "vm-a"),
        (&team, "vm-b"),
        (&other, "vm-a"),
        (&other, "vm-c"),
        (&other, "vm-d"),
        (&apart, "vm-e"),
    ] {
        let (status, joined) = route_request(
            app(&state),
            Method::PUT,
            &format!("/networks/{network}/members/{vm}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let lease = joined["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|member| member["vm_id"] == vm)
            .unwrap()["address"]
            .as_str()
            .unwrap()
            .to_string();
        addresses.insert(vm, lease);
    }
    let ask = |name: Option<&str>, address: Option<&str>| {
        let mut request = json!({ "source_vm": "vm-a", "owner_secret": "secret-a" });
        if let Some(name) = name {
            request["name"] = json!(name);
        }
        if let Some(address) = address {
            request["address"] = json!(address);
        }
        request
    };

    let (status, answer) = private_resolve(&state, ask(Some("beta.team"), None)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["address"], json!(addresses["vm-b"]));
    assert_eq!(answer["name"], "beta.team.capsem.internal");
    assert_eq!(answer["vm"], "vm-b");
    let (status, answer) = private_resolve(&state, ask(Some("Gamma"), None)).await;
    assert_eq!(status, StatusCode::OK, "a short unambiguous name, any case: {answer}");
    assert_eq!(answer["address"], json!(addresses["vm-c"]));
    let (status, _) = private_resolve(&state, ask(Some("beta"), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "beta is in team and other: ambiguous");
    let (status, answer) = private_resolve(&state, ask(Some("beta.other"), None)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["address"], json!(addresses["vm-d"]));
    let (status, _) = private_resolve(&state, ask(Some("echo"), None)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a member of a network the asker is not in"
    );
    let (status, _) = private_resolve(&state, ask(Some("echo.apart"), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = private_resolve(&state, ask(Some("nobody.team"), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, answer) = private_resolve(&state, ask(Some("alpha.team"), None)).await;
    assert_eq!(status, StatusCode::OK, "the asker sees itself: {answer}");

    let (status, answer) = private_resolve(&state, ask(None, Some(&addresses["vm-b"]))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["name"], "beta.team.capsem.internal");
    let (status, _) = private_resolve(&state, ask(None, Some(&addresses["vm-e"]))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an address outside the asker's networks");
    let (status, _) = private_resolve(&state, ask(None, None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = private_resolve(&state, ask(Some("beta.team"), Some(&addresses["vm-b"]))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut forged = ask(Some("beta.team"), None);
    forged["owner_secret"] = json!("wrong");
    assert_eq!(private_resolve(&state, forged).await.0, StatusCode::FORBIDDEN);

    // Leaving takes the name with it at once.
    let (status, _) = route_request(
        app(&state),
        Method::DELETE,
        &format!("/networks/{team}/members/vm-b"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = private_resolve(&state, ask(Some("beta.team"), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, answer) = private_resolve(&state, ask(Some("beta"), None)).await;
    assert_eq!(status, StatusCode::OK, "no longer ambiguous: {answer}");
    assert_eq!(answer["address"], json!(addresses["vm-d"]));
}
