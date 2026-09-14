use super::*;
use std::time::Duration;

#[tokio::test]
async fn disconnect_releases_stream_registration_and_pending_ack() {
    let jobs = Arc::new(JobStore::new());
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let (control, mut commands) = mpsc::channel(1);
    let (output, consumer) = mpsc::channel(1);
    let task = tokio::spawn(run(11, "sleep 100".into(), true, jobs.clone(), control, output, db));
    assert!(matches!(
        commands.recv().await,
        Some(ServiceToProcess::Exec { id: 11, .. })
    ));
    jobs.pending_acks.lock().unwrap().insert(
        11,
        capsem_proto::HostToGuest::Exec {
            id: 11,
            command: "sleep 100".into(),
        },
    );
    drop(consumer);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert!(jobs.jobs.lock().unwrap().is_empty());
    assert!(jobs.active_execs.lock().unwrap().is_empty());
    assert!(jobs.pending_acks.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_dispatch_returns_error_and_removes_registration() {
    let jobs = Arc::new(JobStore::new());
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let (control, commands) = mpsc::channel(1);
    let (output, mut consumer) = mpsc::channel(1);
    drop(commands);
    run(12, "true".into(), true, jobs.clone(), control, output, db).await;
    assert!(matches!(consumer.recv().await, Some(ProcessToService::ExecResult {
        id: 12, exit_code: -1, stderr, ..
    }) if stderr == b"guest control channel closed"));
    assert!(jobs.jobs.lock().unwrap().is_empty());
    assert!(jobs.active_execs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn duplicate_id_does_not_replace_original_job_or_dispatch_again() {
    let jobs = Arc::new(JobStore::new());
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let (original, mut original_result) = oneshot::channel();
    jobs.jobs.lock().unwrap().insert(13, original);
    jobs.active_execs.lock().unwrap().insert(13, ActiveExec::new());
    let (control, mut commands) = mpsc::channel(1);
    let (output, mut consumer) = mpsc::channel(1);
    run(13, "true".into(), true, jobs.clone(), control, output, db).await;
    assert!(matches!(consumer.recv().await, Some(ProcessToService::ExecResult {
        id: 13, exit_code: -1, stderr, ..
    }) if stderr == b"exec id is already in use"));
    assert!(commands.recv().await.is_none());
    assert!(matches!(
        original_result.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(jobs.active_execs.lock().unwrap().contains_key(&13));
}
