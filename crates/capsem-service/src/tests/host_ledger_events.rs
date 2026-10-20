use super::*;

/// The host ledger lives in the home's sessions dir, wherever the run dir is,
/// and the service never opens the dead main.db beside it.
#[test]
fn the_host_ledger_opens_in_the_home_and_leaves_main_db_alone() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("home").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let main_db = sessions.join("main.db");
    std::fs::write(&main_db, b"not a ledger this build can read").unwrap();
    let handle = host_ledger::open_host_ledger(&sessions).unwrap();
    assert_eq!(handle.path(), sessions.join("host.db"));
    drop(handle);
    assert_eq!(std::fs::read(&main_db).unwrap(), b"not a ledger this build can read");
}

/// A forensic record that waits for the next periodic flush can lose a stop
/// to a crash; an outside reader once found a session created and never
/// stopped right after it ended. Each host event is on disk when recorded.
#[tokio::test]
async fn host_events_are_on_disk_when_recorded() {
    let (state, _dir) = make_test_state_with_tempdir();
    state
        .record_host_session_created("s1", Default::default())
        .await
        .unwrap();
    state.record_host_session_stopped("s1", "stopped", false).await.unwrap();
    let reader = capsem_logger::DbReader::open(state.host_ledger.path()).unwrap();
    let kinds: Vec<_> = reader
        .host_events()
        .unwrap()
        .into_iter()
        .filter(|(event, _, _)| event.session_id.as_deref() == Some("s1"))
        .map(|(event, _, _)| event.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            capsem_logger::HostEventKind::SessionCreated,
            capsem_logger::HostEventKind::SessionStopped
        ]
    );
    assert!(reader.verify_host_chain().unwrap() >= 2);
}

#[tokio::test]
async fn host_session_created_records_uuid_id_not_display_name() {
    let (state, _dir) = make_test_state_with_tempdir();
    let id = new_persistent_vm_id();
    uuid::Uuid::parse_str(&id).expect("VM route id should be a UUID");
    state
        .record_host_session_created(&id, Default::default())
        .await
        .unwrap();
    state.host_ledger.flush().await.unwrap();
    let events = state.host_ledger.host_events().await.unwrap();
    let created: Vec<_> = events
        .iter()
        .filter(|event| event.kind == capsem_logger::HostEventKind::SessionCreated)
        .map(|event| event.session_id.clone())
        .collect();
    assert_eq!(created, vec![Some(id)]);
}

#[tokio::test]
async fn handle_stats_folds_the_replayed_host_ledger() {
    use capsem_logger::HostEventKind::{SessionCreated, SessionStopped};
    let (state, _dir) = make_test_state_with_tempdir();
    let mut counters = capsem_proto::ledger_counters::LedgerCounters::default();
    counters.net.total = 50;
    counters.model.total.input_tokens = 10_000;
    counters.model.total.cost_micro_usd = 420_000;
    let created = capsem_proto::host_session::HostSessionDetail::default();
    let stopped = host_ledger::stopped_detail("stopped", Some(counters));
    for event in [
        host_ledger::host_session_event(SessionCreated, "20260412-120000-abcd", 1_000, &created),
        host_ledger::host_session_event(SessionStopped, "20260412-120000-abcd", 2_000, &stopped),
    ] {
        state
            .host_ledger
            .write(capsem_logger::WriteOp::HostEvent(event))
            .await
            .unwrap();
    }
    state.host_ledger.flush().await.unwrap();
    state.hydrate_host_stats().await.unwrap();

    let response = handle_stats(State(state)).await.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(resp["global"]["total_sessions"], 1);
    assert_eq!(resp["global"]["total_input_tokens"], 10000);
    assert_eq!(resp["global"]["total_estimated_cost"], 0.42);
    assert_eq!(resp["global"]["total_requests"], 50);
    assert_eq!(resp["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(resp["sessions"][0]["id"], "20260412-120000-abcd");
    assert_eq!(resp["sessions"][0]["status"], "stopped");
}
