use super::*;
use std::time::Duration;

#[tokio::test]
async fn owner_cancellation_releases_stream_registration_and_pending_ack() {
    let jobs = Arc::new(JobStore::new());
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let (control, mut commands) = mpsc::channel(1);
    let (output, consumer) = mpsc::channel(1);
    let registration = install(11, true, &jobs, &output);
    let task = tokio::spawn(run(
        11,
        "sleep 100".into(),
        jobs.clone(),
        control.clone(),
        output,
        db,
        registration,
    ));
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
    cancel(11, &jobs, &control).await.unwrap();
    assert!(matches!(
        commands.recv().await,
        Some(ServiceToProcess::CancelExec { id: 11 })
    ));
    jobs.jobs
        .lock()
        .unwrap()
        .remove(&11)
        .unwrap()
        .send(JobResult::Exec {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: 143,
            truncated: false,
        })
        .unwrap();
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
    let registration = install(12, true, &jobs, &output);
    run(12, "true".into(), jobs.clone(), control, output, db, registration).await;
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
    let registration = install(13, true, &jobs, &output);
    assert!(registration.is_none(), "a duplicate id must not be registered again");
    run(13, "true".into(), jobs.clone(), control, output, db, registration).await;
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

/// The IPC read loop handles stdin inline. A full stdin queue used to park
/// that loop, so CancelExec on the same connection was never read and a
/// command ignoring stdin could not be cancelled. Overflow is refused at once.
#[test]
fn stdin_beyond_the_window_is_refused_without_waiting() {
    let jobs = JobStore::new();
    jobs.active_execs.lock().unwrap().insert(21, ActiveExec::new());
    let frame = || capsem_proto::ExecInputFrame::Data(b"x".to_vec());
    for _ in 0..capsem_proto::EXEC_STDIN_WINDOW {
        input(21, frame(), &jobs).unwrap();
    }
    let refused = input(21, frame(), &jobs).unwrap_err();
    assert!(refused.contains("window"), "{refused}");
}

/// A client may send stdin immediately after the stream start. Registering the
/// exec inside the spawned task left a window where that first frame was
/// refused as "exec is not running" and silently dropped.
#[tokio::test]
async fn stdin_sent_immediately_after_the_start_reaches_the_exec() {
    let jobs = Arc::new(JobStore::new());
    let (output, _consumer) = mpsc::channel(4);
    let registration = install(31, true, &jobs, &output).expect("a fresh id registers");

    input(31, capsem_proto::ExecInputFrame::Data(b"hello\n".to_vec()), &jobs)
        .expect("stdin that follows the start frame is queued");

    let mut queued = jobs
        .active_execs
        .lock()
        .unwrap()
        .get_mut(&31)
        .and_then(|active| active.input_rx.take())
        .expect("the exec owns its stdin queue");
    assert_eq!(
        queued.try_recv().unwrap(),
        capsem_proto::ExecInputFrame::Data(b"hello\n".to_vec())
    );
    drop(registration);
}
