use super::*;
use std::time::Duration;

#[tokio::test]
async fn snapshot_replies_require_the_exact_operation_and_phase() {
    let jobs = Arc::new(JobStore::new());
    let (host, mut guest) = crate::guest_control::channel(4);
    let pending = {
        let jobs = Arc::clone(&jobs);
        tokio::spawn(async move { snapshot_transition(&host, &jobs, Duration::from_secs(1), false).await })
    };
    let HostToGuest::SnapshotTransition { id, frozen: false } = guest.recv().await.unwrap() else {
        panic!("wrong request")
    };
    jobs.finish_snapshot_transition(id - 1, false, None);
    jobs.finish_snapshot_transition(id, true, None);
    jobs.finish_snapshot_transition(id, false, None);
    pending.await.unwrap().unwrap();
}

#[tokio::test]
async fn snapshot_timeout_and_cancellation_release_waiters_and_reject_late_replies() {
    let jobs = Arc::new(JobStore::new());
    let (host, mut guest) = crate::guest_control::channel(4);
    let first = snapshot_transition(&host, &jobs, Duration::from_millis(10), true).await;
    assert!(first.unwrap_err().to_string().contains("timed out"));
    let HostToGuest::SnapshotTransition { id: old, .. } = guest.recv().await.unwrap() else {
        panic!("wrong request")
    };
    let second = {
        let jobs = Arc::clone(&jobs);
        let host = host.clone();
        tokio::spawn(async move { snapshot_transition(&host, &jobs, Duration::from_secs(1), false).await })
    };
    let HostToGuest::SnapshotTransition { id, .. } = guest.recv().await.unwrap() else {
        panic!("wrong request")
    };
    assert_ne!(id, old);
    jobs.finish_snapshot_transition(old, true, None);
    second.abort();
    assert!(second.await.unwrap_err().is_cancelled());
    let third = {
        let jobs = Arc::clone(&jobs);
        tokio::spawn(async move { snapshot_transition(&host, &jobs, Duration::from_secs(1), false).await })
    };
    let HostToGuest::SnapshotTransition { id: latest, .. } = guest.recv().await.unwrap() else {
        panic!("wrong request")
    };
    jobs.finish_snapshot_transition(id, false, None);
    jobs.finish_snapshot_transition(latest, false, Some("ext4 still frozen".into()));
    assert!(third
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("ext4 still frozen"));
}

#[tokio::test]
async fn snapshot_ack_deadline_includes_a_full_control_queue() {
    let jobs = Arc::new(JobStore::new());
    let (host, _guest) = crate::guest_control::channel(1);
    host.send(HostToGuest::Shutdown).await.unwrap();
    assert!(snapshot_transition(&host, &jobs, Duration::from_millis(10), false)
        .await
        .is_err());
    assert!(jobs.snapshot_transition.lock().unwrap().is_none());
}
