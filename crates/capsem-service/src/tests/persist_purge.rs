use super::*;

fn insert_ephemeral_instance(state: &ServiceState, id: &str) -> PathBuf {
    let session_dir = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("marker"), id.as_bytes()).unwrap();
    insert_fake_instance_with_session_dir(state, id, std::process::id(), session_dir.clone());
    session_dir
}

async fn persist(state: &Arc<ServiceState>, id: &str, name: &str) -> Result<Json<serde_json::Value>, AppError> {
    handle_persist(
        State(Arc::clone(state)),
        Path(id.to_string()),
        Json(PersistRequest { name: name.to_string() }),
    )
    .await
}

fn assert_still_ephemeral(state: &ServiceState, id: &str, session_dir: &StdPath) {
    assert!(
        session_dir.join("marker").is_file(),
        "loser's session dir must stay where its process is running: {}",
        session_dir.display()
    );
    assert!(
        !state.run_dir.join("persistent").join(id).exists(),
        "loser must not leave an orphaned persistent/{id} directory"
    );
    let (persistent, live_dir) = state
        .instances
        .lock()
        .unwrap()
        .get(id)
        .map(|info| (info.persistent, info.session_dir.clone()))
        .expect("loser instance still registered");
    assert!(!persistent, "loser must still be reported as ephemeral");
    assert_eq!(
        live_dir, session_dir,
        "loser's InstanceInfo must still point at its live dir"
    );
}

#[tokio::test]
async fn persist_rolls_the_session_dir_back_when_registration_fails() {
    // `register` refuses a duplicate id as well as a duplicate name. Before
    // the claim was atomic, the session dir had already been renamed under
    // persistent/<id> by the time that refusal arrived, and nothing moved it
    // back: no registry entry, an InstanceInfo still saying persistent:false
    // and pointing at a directory that no longer existed.
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = insert_ephemeral_instance(&state, "persist-src");
    let mut clash = test_persistent_entry("other-name", state.run_dir.join("persistent").join("elsewhere"));
    clash.id = "persist-src".to_string();
    state.persistent_registry.lock().unwrap().register(clash).unwrap();

    let error = persist(&state, "persist-src", "fresh-name")
        .await
        .expect_err("duplicate id must be refused");
    assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR, "{}", error.1);

    assert_still_ephemeral(&state, "persist-src", &session_dir);
    let (claimed, entries) = {
        let registry = state.persistent_registry.lock().unwrap();
        (registry.contains("fresh-name"), registry.list().count())
    };
    assert!(!claimed);
    assert_eq!(entries, 1, "only the pre-existing entry remains");
}

#[tokio::test]
async fn racing_persists_on_one_name_leave_one_entry_and_no_orphan() {
    // Every handler runs synchronously up to the claim, which it hands to the
    // blocking pool; joining them on one runtime thread therefore races the
    // four claims themselves, each on its own thread against one registry.
    // (The test profile override is thread-local, so the handlers cannot be
    // spread over worker threads.)
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let ids = ["race-a", "race-b", "race-c", "race-d"];
    let dirs: Vec<PathBuf> = ids.iter().map(|id| insert_ephemeral_instance(&state, id)).collect();

    let outcomes = futures::future::join_all(ids.iter().map(|id| persist(&state, id, "shared"))).await;
    let mut winners = Vec::new();
    let mut losers = Vec::new();
    for (id, outcome) in ids.iter().zip(outcomes) {
        let id = id.to_string();
        match outcome {
            Ok(_) => winners.push(id),
            Err(error) => {
                assert_eq!(
                    error.0,
                    StatusCode::CONFLICT,
                    "loser must see a name conflict: {}",
                    error.1
                );
                losers.push(id);
            }
        }
    }
    assert_eq!(
        winners.len(),
        1,
        "exactly one persist may win the name: {winners:?} / {losers:?}"
    );
    assert_eq!(losers.len(), 3);

    let winner = &winners[0];
    let (entries, entry_id, entry_dir) = {
        let registry = state.persistent_registry.lock().unwrap();
        let entry = registry.get("shared").expect("winner registered under the shared name");
        (registry.list().count(), entry.id.clone(), entry.session_dir.clone())
    };
    assert_eq!(entries, 1);
    assert_eq!(&entry_id, winner);
    assert_eq!(entry_dir, state.run_dir.join("persistent").join(winner));
    assert!(entry_dir.join("marker").is_file());
    let (persistent, name) = state
        .instances
        .lock()
        .unwrap()
        .get(winner)
        .map(|info| (info.persistent, info.name.clone()))
        .unwrap();
    assert!(persistent);
    assert_eq!(name, "shared");
    for (id, session_dir) in ids.iter().zip(dirs) {
        if id != winner {
            assert_still_ephemeral(&state, id, &session_dir);
        }
    }
}

fn register_defunct_entry(state: &ServiceState, name: &str, session_dir: PathBuf) {
    let mut entry = test_persistent_entry(name, session_dir);
    entry.defunct = true;
    state.persistent_registry.lock().unwrap().register(entry).unwrap();
}

async fn purge_all(state: &Arc<ServiceState>, all: bool) -> PurgeResponse {
    handle_purge(State(Arc::clone(state)), Json(PurgeRequest { all }))
        .await
        .expect("purge completes")
        .0
}

#[tokio::test]
async fn purge_removes_a_defunct_session_under_the_persistent_root() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent").join("gone");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("marker"), b"x").unwrap();
    register_defunct_entry(&state, "gone", session_dir.clone());

    let response = purge_all(&state, false).await;
    assert_eq!(response.persistent_purged, 1);
    assert!(!session_dir.exists());
    assert!(!state.persistent_registry.lock().unwrap().contains("gone"));
}

#[tokio::test]
async fn purge_refuses_a_registry_session_dir_outside_the_run_roots() {
    // The registry is a JSON file under the user's home; a session_dir it
    // reports is data, not authority. Purge used to hand it straight to
    // remove_dir_all.
    let (state, dir) = make_test_state_with_tempdir();
    let outside = dir.path().join("outside-session");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("marker"), b"keep").unwrap();
    register_defunct_entry(&state, "escaped", outside.clone());

    let response = purge_all(&state, true).await;
    assert_eq!(response.persistent_purged, 0, "a refused delete is not a purge");
    assert!(
        outside.join("marker").is_file(),
        "purge must not delete outside the service roots"
    );
    assert!(
        state.persistent_registry.lock().unwrap().contains("escaped"),
        "an entry whose directory was not removed stays visible rather than silently vanishing"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn purge_refuses_a_symlinked_session_dir_under_the_persistent_root() {
    let (state, dir) = make_test_state_with_tempdir();
    let outside = dir.path().join("outside-target");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("marker"), b"keep").unwrap();
    let persistent_root = state.run_dir.join("persistent");
    std::fs::create_dir_all(&persistent_root).unwrap();
    let link = persistent_root.join("linked");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    register_defunct_entry(&state, "linked", link.clone());

    let response = purge_all(&state, false).await;
    assert_eq!(response.persistent_purged, 0);
    assert!(
        outside.join("marker").is_file(),
        "purge must not follow a symlink out of the root"
    );
}
