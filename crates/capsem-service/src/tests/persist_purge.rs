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

fn registry_entry_dir(state: &ServiceState, name: &str) -> Option<PathBuf> {
    state
        .persistent_registry
        .lock()
        .unwrap()
        .get(name)
        .map(|entry| entry.session_dir.clone())
}

/// A running capsem-process holds its session directory by path: the
/// VirtioFS workspace, the auto-snapshot scheduler, the MCP file tools and
/// every lazily opened session.db reader all name `sessions/<id>/...`.
/// Renaming that directory under a live process left snapshots, file tools
/// and history failing until the next restart. Persist now claims the name
/// and registers the directory where it is; the move to `persistent/<id>`
/// happens when the process has exited.
#[tokio::test]
async fn persist_keeps_the_live_session_dir_while_the_process_runs() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = insert_ephemeral_instance(&state, "live-src");

    let _ = persist(&state, "live-src", "kept").await.expect("persist succeeds");

    assert!(session_dir.join("marker").is_file(), "the live dir must not move");
    assert!(
        !state.run_dir.join("persistent").join("live-src").exists(),
        "nothing may be renamed under persistent/ while the process runs"
    );
    assert_eq!(
        registry_entry_dir(&state, "kept").as_deref(),
        Some(session_dir.as_path())
    );
    let (persistent, name, live_dir) = state
        .instances
        .lock()
        .unwrap()
        .get("live-src")
        .map(|info| (info.persistent, info.name.clone(), info.session_dir.clone()))
        .expect("instance still registered");
    assert!(persistent);
    assert_eq!(name, "kept");
    assert_eq!(live_dir, session_dir);
}

#[tokio::test]
async fn persist_refuses_a_duplicate_id_and_leaves_the_instance_ephemeral() {
    // `register` refuses a duplicate id as well as a duplicate name. The
    // refusal must leave no trace: no entry, no moved directory, and an
    // InstanceInfo that still says ephemeral.
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
    assert_eq!(entry_dir, state.run_dir.join("sessions").join(winner));
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

/// Register `name` as a persisted session still living where it ran, with a
/// fake running instance, and reap a child that exits with `code`.
async fn reap_persisted_live_session(state: &Arc<ServiceState>, id: &str, name: &str, code: i32) -> PathBuf {
    let live_dir = insert_ephemeral_instance(state, id);
    let mut entry = test_persistent_entry(name, live_dir.clone());
    entry.id = id.to_string();
    state.persistent_registry.lock().unwrap().register(entry).unwrap();
    if let Some(info) = state.instances.lock().unwrap().get_mut(id) {
        info.persistent = true;
        info.name = name.to_string();
    }
    let child = tokio::process::Command::new("sh")
        .args(["-c", &format!("exit {code}")])
        .spawn()
        .expect("spawn child");
    state.instances.lock().unwrap().get_mut(id).unwrap().pid = child.id().unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        id.to_string(),
        name.to_string(),
        Arc::clone(state),
        state.run_dir.join("instances").join(format!("{id}.sock")),
        live_dir.clone(),
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), reaper)
        .await
        .unwrap()
        .unwrap();
    assert!(!state.instances.lock().unwrap().contains_key(id));
    live_dir
}

#[tokio::test]
async fn the_exit_reaper_moves_a_persisted_live_dir_under_persistent() {
    let (state, _dir) = make_test_state_with_tempdir();
    let live_dir = reap_persisted_live_session(&state, "settle-src", "settled", 0).await;

    let target = state.run_dir.join("persistent").join("settle-src");
    assert_eq!(
        registry_entry_dir(&state, "settled").as_deref(),
        Some(target.as_path()),
        "the entry must follow the directory"
    );
    assert!(target.join("marker").is_file(), "the session bytes moved with it");
    assert!(!live_dir.exists(), "nothing stays behind under sessions/");
    let (defunct, suspended) = state
        .persistent_registry
        .lock()
        .unwrap()
        .get("settled")
        .map(|entry| (entry.defunct, entry.suspended))
        .unwrap();
    assert!(!defunct);
    assert!(!suspended);
}

#[tokio::test]
async fn a_failed_settle_keeps_the_entry_on_the_live_dir() {
    // `persistent/<id>` already holds something: the rename fails and the
    // session must stay reachable where it is rather than vanish.
    let (state, _dir) = make_test_state_with_tempdir();
    let occupied = state.run_dir.join("persistent").join("blocked-src");
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("squatter"), b"x").unwrap();

    let live_dir = reap_persisted_live_session(&state, "blocked-src", "blocked", 3).await;

    assert_eq!(
        registry_entry_dir(&state, "blocked").as_deref(),
        Some(live_dir.as_path())
    );
    assert!(live_dir.join("marker").is_file());
    assert!(occupied.join("squatter").is_file(), "the occupant is not overwritten");
    let defunct = state
        .persistent_registry
        .lock()
        .unwrap()
        .get("blocked")
        .map(|entry| entry.defunct)
        .unwrap();
    assert!(
        defunct,
        "crash bookkeeping still runs against the directory that exists"
    );
}

#[test]
fn settle_is_a_no_op_for_a_directory_already_under_persistent() {
    let (state, _dir) = make_test_state_with_tempdir();
    let home = state.run_dir.join("persistent").join("home-id");
    std::fs::create_dir_all(&home).unwrap();
    let mut entry = test_persistent_entry("home", home.clone());
    entry.id = "home-id".to_string();
    state.persistent_registry.lock().unwrap().register(entry).unwrap();

    let settled = crate::vm_lifecycle::settle_persistent_session_dir(&state, "home", &home);
    assert_eq!(settled, home);
    assert!(home.is_dir());
    let unknown = crate::vm_lifecycle::settle_persistent_session_dir(&state, "no-such-name", &home);
    assert_eq!(
        unknown, home,
        "an unregistered name settles to the directory it was given"
    );
}

/// A resume must settle before it launches: the reaper of the previous
/// process may not have run yet, and a process started from `sessions/<id>`
/// would lose its directory to that reaper's move.
#[test]
fn resume_settles_the_session_dir_before_spawning() {
    let source = include_str!("../main.rs");
    let start = source.find("    fn resume_sandbox(").expect("resume_sandbox exists");
    let end = start + source[start..].find("    fn has_existing_resume_checkpoint(").unwrap();
    let body = &source[start..end];
    let settle = body
        .find("settle_persistent_session_dir(")
        .expect("resume settles the session dir");
    let spawn = body.find("Command::new(").expect("resume spawns capsem-process");
    assert!(settle < spawn, "resume_sandbox must settle before it spawns the child");
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
