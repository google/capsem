use super::*;

/// The identity contract for every session-enumerating surface: a persistent
/// VM is one session, addressed by its opaque session id, whether or not it
/// is running. Registry entries are keyed by display name, and a running
/// persistent VM is also in `instances` under its id, so keying one source by
/// name and the other by id counted the same session twice and registered a
/// DB handle under the name key that nothing ever unregistered.
const VM_ID: &str = "22222222-2222-4222-8222-222222222222";
const DISPLAY_NAME: &str = "co-work1";

async fn write_one_security_event(db_path: PathBuf) {
    tokio::task::spawn_blocking(move || {
        let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
        writer.write_blocking(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_456,
                "abcdef123456",
                "http.request",
                "profiles.rules.default_http",
                r#"{"name":"default_http"}"#,
                r#"{"event_type":"http.request"}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow)
            .with_detection_level(capsem_logger::SecurityDetectionLevel::Informational),
        ));
        writer.shutdown_blocking();
    })
    .await
    .unwrap();
}

fn persistent_session_dir(state: &ServiceState) -> PathBuf {
    let session_dir = state.run_dir.join("persistent").join(VM_ID);
    std::fs::create_dir_all(&session_dir).unwrap();
    session_dir
}

fn register_persistent_entry(state: &ServiceState, session_dir: &StdPath) {
    let mut entry = test_persistent_entry(DISPLAY_NAME, session_dir.to_path_buf());
    entry.id = VM_ID.to_string();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(DISPLAY_NAME.to_string(), entry);
}

fn insert_running_persistent_instance(state: &ServiceState, session_dir: &StdPath) {
    insert_fake_instance_with_session_dir(state, VM_ID, std::process::id(), session_dir.to_path_buf());
    match state.instances.lock().unwrap().get_mut(VM_ID) {
        Some(info) => {
            info.name = DISPLAY_NAME.to_string();
            info.persistent = true;
        }
        None => panic!("instance {VM_ID} was just inserted"),
    }
}

fn session_db_handle_keys(state: &ServiceState) -> Vec<String> {
    let mut keys: Vec<String> = state.session_db_handles.lock().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

#[tokio::test]
async fn running_persistent_vm_is_one_session_on_service_wide_ledger_routes() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = persistent_session_dir(&state);
    register_persistent_entry(&state, &session_dir);
    insert_running_persistent_instance(&state, &session_dir);
    write_one_security_event(session_dir.join("session.db")).await;
    let app = build_service_router(Arc::clone(&state));

    let (status, security) = route_request(app.clone(), axum::http::Method::GET, "/security/status", None).await;
    assert_eq!(status, StatusCode::OK, "{security}");
    assert_eq!(security["total"], 1, "one event must count once: {security}");
    assert_eq!(security["sessions"].as_array().map(Vec::len), Some(1), "{security}");
    assert_eq!(security["sessions"][0]["vm_id"], VM_ID, "{security}");

    let (status, detection) = route_request(app, axum::http::Method::GET, "/detection/status", None).await;
    assert_eq!(status, StatusCode::OK, "{detection}");
    assert_eq!(detection["total"], 1, "{detection}");
    assert_eq!(detection["sessions"].as_array().map(Vec::len), Some(1), "{detection}");
    assert_eq!(detection["sessions"][0]["vm_id"], VM_ID, "{detection}");

    assert_eq!(
        session_db_handle_keys(&state),
        vec![VM_ID.to_string()],
        "the ledger routes must register exactly one DB handle, keyed by session id; a handle under the \
         display name is never unregistered when the VM stops"
    );
}

#[test]
fn session_dir_enumeration_keys_persistent_entries_by_session_id() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = persistent_session_dir(&state);
    register_persistent_entry(&state, &session_dir);

    // Stopped: the registry is the only source, and it still speaks id.
    assert_eq!(
        service_session_dirs(&state),
        vec![(VM_ID.to_string(), session_dir.clone())]
    );
    assert_eq!(
        profile_session_dirs(&state, "code"),
        vec![(VM_ID.to_string(), session_dir.clone())]
    );
    assert!(profile_session_dirs(&state, "not-the-profile").is_empty());

    // Running: the instance and the registry entry collapse into one row.
    insert_running_persistent_instance(&state, &session_dir);
    assert_eq!(
        service_session_dirs(&state),
        vec![(VM_ID.to_string(), session_dir.clone())]
    );
    assert_eq!(
        profile_session_dirs(&state, "code"),
        vec![(VM_ID.to_string(), session_dir)]
    );
}

#[test]
fn session_dir_enumeration_falls_back_to_the_session_dir_basename_for_legacy_entries() {
    // A registry written before entries carried ids has an empty `id`; the
    // session directory basename is the id those sessions were created under.
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent").join("legacy-session");
    std::fs::create_dir_all(&session_dir).unwrap();
    let mut entry = test_persistent_entry("legacy-name", session_dir.clone());
    entry.id = String::new();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("legacy-name".to_string(), entry);

    assert_eq!(
        service_session_dirs(&state),
        vec![("legacy-session".to_string(), session_dir)]
    );
}

#[tokio::test]
async fn startup_hydration_keys_persistent_db_handles_by_session_id() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = persistent_session_dir(&state);
    register_persistent_entry(&state, &session_dir);
    write_one_security_event(session_dir.join("session.db")).await;

    state.hydrate_session_db_handles();
    assert_eq!(session_db_handle_keys(&state), vec![VM_ID.to_string()]);
    assert!(
        state.session_db_handle(DISPLAY_NAME).is_none(),
        "display name is not a handle key"
    );

    // A running persistent VM must not gain a second handle on rehydration.
    insert_running_persistent_instance(&state, &session_dir);
    state.hydrate_session_db_handles();
    assert_eq!(session_db_handle_keys(&state), vec![VM_ID.to_string()]);
}

#[test]
fn persistent_route_identity_source_guard() {
    let sources = [
        ("main.rs", include_str!("../main.rs")),
        ("ledger_routes.rs", include_str!("../ledger_routes.rs")),
    ];
    for (file, source) in sources {
        for forbidden in [
            "registry.get(&id)",
            "registry.get(id)",
            "registry.get_mut(&id)",
            "registry.get_mut(id)",
            "registry.unregister(&id)",
            "instances.contains_key(&entry.name)",
            "SandboxInfo::new(\n            entry.name.clone()",
            // Registry maps are keyed by display name. Route code reaches the
            // registry through `list()` and the identity helpers; iterating
            // the raw map and treating its key as a session id is the
            // double-count bug.
            "registry.data.vms.",
        ] {
            assert!(
                !source.contains(forbidden),
                "{file}: {forbidden} reintroduced the VM identity footgun: route `id` is the opaque session id; \
                 registry `name` is display/resume identity only"
            );
        }
    }
}
