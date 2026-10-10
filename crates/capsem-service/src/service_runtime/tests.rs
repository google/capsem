use super::*;

#[test]
fn gateway_readiness_requires_complete_stamped_runtime_files() {
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("gateway.token");
    let port = dir.path().join("gateway.port");
    let preview = dir.path().join("preview.port");
    for path in [&token, &port, &preview] {
        std::fs::write(path, "").unwrap();
    }
    assert!(!gateway_runtime_ready(&token, &port, &preview));

    std::fs::write(&token, "a".repeat(64)).unwrap();
    std::fs::write(&port, "19222").unwrap();
    std::fs::write(&preview, "19223").unwrap();
    assert!(gateway_runtime_ready(&token, &port, &preview));

    std::fs::write(&token, "short").unwrap();
    assert!(!gateway_runtime_ready(&token, &port, &preview));
    std::fs::write(&token, "a".repeat(64)).unwrap();
    std::fs::write(&preview, "0").unwrap();
    assert!(!gateway_runtime_ready(&token, &port, &preview));
}

#[test]
fn shutdown_collects_only_quiesced_ephemeral_sessions() {
    let state = crate::tests::make_test_state_owned();
    assert!(kill_all_vm_processes(&state).is_empty());

    let sessions = state.run_dir.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let mut ephemeral = crate::tests::test_instance();
    ephemeral.id = "ephemeral".into();
    ephemeral.pid = 0;
    ephemeral.uds_path = state.run_dir.join("ephemeral.sock");
    ephemeral.session_dir = sessions.join(&ephemeral.id);
    std::fs::create_dir(&ephemeral.session_dir).unwrap();
    for path in [
        ephemeral.uds_path.clone(),
        ephemeral.uds_path.with_extension("ready"),
        ephemeral.uds_path.with_extension("launched"),
    ] {
        std::fs::write(path, b"sentinel").unwrap();
    }

    let mut persistent = crate::tests::test_instance();
    persistent.id = "persistent".into();
    persistent.pid = 0;
    persistent.persistent = true;
    persistent.uds_path = state.run_dir.join("persistent.sock");
    persistent.session_dir = state.run_dir.join("persistent").join(&persistent.id);
    std::fs::create_dir_all(&persistent.session_dir).unwrap();

    state
        .instances
        .lock()
        .unwrap()
        .extend([(ephemeral.id.clone(), ephemeral), (persistent.id.clone(), persistent)]);

    assert_eq!(
        kill_all_vm_processes(&state),
        vec![("ephemeral".into(), sessions.join("ephemeral"))]
    );
    assert!(!state.run_dir.join("ephemeral.sock").exists());
    assert!(!state.run_dir.join("ephemeral.ready").exists());
    assert!(!state.run_dir.join("ephemeral.launched").exists());
    assert!(sessions.join("ephemeral").exists());
    assert!(state.run_dir.join("persistent").join("persistent").exists());
}

#[test]
fn shutdown_deletes_only_contained_ephemeral_sessions() {
    let state = crate::tests::make_test_state_owned();
    let sessions = state.run_dir.join("sessions");
    let contained = sessions.join("contained");
    std::fs::create_dir_all(&contained).unwrap();
    std::fs::write(contained.join("session.db"), b"ledger").unwrap();

    let outside = state.run_dir.parent().unwrap().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let deleted = delete_shutdown_ephemeral_sessions(
        &state,
        vec![
            ("contained".into(), contained.clone()),
            ("outside".into(), outside.clone()),
        ],
    );

    assert_eq!(deleted, vec!["contained"]);
    assert!(!contained.exists());
    assert!(
        outside.exists(),
        "shutdown deletion must stay within service-owned roots"
    );
}
