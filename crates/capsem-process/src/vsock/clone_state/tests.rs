use super::*;

#[tokio::test]
async fn frozen_clone_waits_for_the_correlated_coordinator_result() {
    let jobs = Arc::new(JobStore::new());
    let (events, mut listener) = broadcast::channel(4);
    let waiting = {
        let jobs = Arc::clone(&jobs);
        let events = events.clone();
        tokio::spawn(async move { await_coordinator_copy(&jobs, &events, 41).await })
    };

    assert!(matches!(
        listener.recv().await.unwrap(),
        ProcessToService::CloneStateReady { id: 41 }
    ));
    jobs.complete_clone(41, Ok(8192)).unwrap();
    assert_eq!(waiting.await.unwrap().unwrap(), 8192);
    assert!(jobs.clone_completions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn clone_without_a_coordinator_listener_keeps_no_completion_slot() {
    let jobs = JobStore::new();
    let (events, listener) = broadcast::channel(1);
    drop(listener);
    let error = await_coordinator_copy(&jobs, &events, 7).await.unwrap_err();
    assert!(error.to_string().contains("no coordinator listener"));
    assert!(jobs.clone_completions.lock().unwrap().is_empty());
}
