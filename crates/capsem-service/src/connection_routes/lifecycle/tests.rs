use super::*;

#[tokio::test]
async fn credential_lifecycle_deletion_rechecks_replacement_after_waiting_for_launch() {
    let state = crate::tests::make_test_state();
    let id = capsem_crypto::uuid_v4().to_string();
    let launching = state.lifecycle.vz.read().await;
    let mut pending = Box::pin(deletion(&state, &id));
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut pending)
        .await
        .is_err());
    let directory = state.run_dir.join("sessions").join(&id);
    crate::tests::insert_fake_instance_with_session_dir(&state, &id, std::process::id(), directory);
    let generation = state.instances.lock().unwrap()[&id].generation;
    drop(launching);
    match pending.await {
        Err(error) => assert_eq!(error.status, StatusCode::CONFLICT),
        Ok(_) => panic!("deletion acquired authority over a replacement"),
    }
    assert_eq!(state.instances.lock().unwrap()[&id].generation, generation);
}
