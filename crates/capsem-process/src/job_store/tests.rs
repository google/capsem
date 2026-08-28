use super::*;

// -----------------------------------------------------------------------
// JobStore
// -----------------------------------------------------------------------

#[test]
fn job_store_insert_and_remove() {
    let store = JobStore::new();
    let (tx, _rx) = oneshot::channel::<JobResult>();
    store.jobs.lock().unwrap().insert(1, tx);
    assert!(store.jobs.lock().unwrap().contains_key(&1));
    let removed = store.jobs.lock().unwrap().remove(&1);
    assert!(removed.is_some());
    assert!(!store.jobs.lock().unwrap().contains_key(&1));
}

#[test]
fn job_store_missing_id_returns_none() {
    let store = JobStore::new();
    let removed = store.jobs.lock().unwrap().remove(&999);
    assert!(removed.is_none());
}

#[test]
fn job_store_concurrent_ids_unique() {
    let store = JobStore::new();
    for i in 0..100 {
        let (tx, _rx) = oneshot::channel::<JobResult>();
        store.jobs.lock().unwrap().insert(i, tx);
    }
    assert_eq!(store.jobs.lock().unwrap().len(), 100);
}

#[test]
fn job_store_active_exec_set_and_clear() {
    let store = JobStore::new();
    assert!(store.active_execs.lock().unwrap().is_empty());

    store
        .active_execs
        .lock()
        .unwrap()
        .insert(42, ActiveExec::new());
    {
        let guard = store.active_execs.lock().unwrap();
        let active = guard.get(&42).unwrap();
        assert!(active.captured.is_empty());
    }

    store.active_execs.lock().unwrap().remove(&42);
    assert!(store.active_execs.lock().unwrap().is_empty());
}

#[test]
fn job_store_active_exec_captures_data() {
    let store = JobStore::new();
    store
        .active_execs
        .lock()
        .unwrap()
        .insert(1, ActiveExec::new());
    if let Some(active) = store.active_execs.lock().unwrap().get_mut(&1) {
        active.captured.extend_from_slice(b"hello ");
        active.captured.extend_from_slice(b"world");
    }
    let captured = store
        .active_execs
        .lock()
        .unwrap()
        .get(&1)
        .unwrap()
        .captured
        .clone();
    assert_eq!(captured, b"hello world");
}

#[test]
fn job_store_overwrite_same_id() {
    let store = JobStore::new();
    let (tx1, _rx1) = oneshot::channel::<JobResult>();
    let (tx2, _rx2) = oneshot::channel::<JobResult>();
    store.jobs.lock().unwrap().insert(1, tx1);
    // Overwriting drops the old sender
    store.jobs.lock().unwrap().insert(1, tx2);
    assert_eq!(store.jobs.lock().unwrap().len(), 1);
}

// -----------------------------------------------------------------------
// JobResult variants
// -----------------------------------------------------------------------

#[test]
fn job_result_exec_fields() {
    let r = JobResult::Exec {
        stdout: b"output".to_vec(),
        stderr: b"err".to_vec(),
        exit_code: 0,
        truncated: false,
    };
    match r {
        JobResult::Exec {
            stdout,
            stderr,
            exit_code,
            ..
        } => {
            assert_eq!(stdout, b"output");
            assert_eq!(stderr, b"err");
            assert_eq!(exit_code, 0);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn job_result_exec_nonzero_exit() {
    let r = JobResult::Exec {
        stdout: vec![],
        stderr: b"command not found".to_vec(),
        exit_code: 127,
        truncated: false,
    };
    match r {
        JobResult::Exec { exit_code, .. } => assert_eq!(exit_code, 127),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn job_result_write_file_success() {
    let r = JobResult::WriteFile {
        success: true,
        error: None,
    };
    match r {
        JobResult::WriteFile { success, error } => {
            assert!(success);
            assert!(error.is_none());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn job_result_write_file_error() {
    let r = JobResult::WriteFile {
        success: false,
        error: Some("permission denied".into()),
    };
    match r {
        JobResult::WriteFile { success, error } => {
            assert!(!success);
            assert_eq!(error.unwrap(), "permission denied");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn job_result_read_file_with_data() {
    let r = JobResult::ReadFile {
        data: Some(b"contents".to_vec()),
        error: None,
    };
    match r {
        JobResult::ReadFile { data, error } => {
            assert_eq!(data.unwrap(), b"contents");
            assert!(error.is_none());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn job_result_read_file_not_found() {
    let r = JobResult::ReadFile {
        data: None,
        error: Some("not found".into()),
    };
    match r {
        JobResult::ReadFile { data, error } => {
            assert!(data.is_none());
            assert_eq!(error.unwrap(), "not found");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn job_result_error() {
    let r = JobResult::Error {
        message: "internal failure".into(),
    };
    match r {
        JobResult::Error { message } => assert_eq!(message, "internal failure"),
        _ => panic!("wrong variant"),
    }
}

// -----------------------------------------------------------------------
// fail_all drains every pending oneshot with an Error
// -----------------------------------------------------------------------

#[tokio::test]
async fn fail_all_resolves_every_pending_oneshot() {
    let job_store = Arc::new(JobStore::new());
    // Register three pending jobs + a snapshot_ready waiter.
    let (tx1, rx1) = oneshot::channel::<JobResult>();
    let (tx2, rx2) = oneshot::channel::<JobResult>();
    let (tx3, rx3) = oneshot::channel::<JobResult>();
    let (snap_tx, snap_rx) = oneshot::channel::<()>();
    {
        let mut jobs = job_store.jobs.lock().unwrap();
        jobs.insert(1, tx1);
        jobs.insert(2, tx2);
        jobs.insert(3, tx3);
    }
    *job_store.snapshot_ready.lock().unwrap() = Some(snap_tx);
    let first_deposited = {
        let mut active = ActiveExec::new();
        active.captured = b"first".to_vec();
        let deposited = Arc::clone(&active.deposited);
        job_store.active_execs.lock().unwrap().insert(1, active);
        deposited
    };
    let second_deposited = {
        let mut active = ActiveExec::new();
        active.captured = b"second".to_vec();
        let deposited = Arc::clone(&active.deposited);
        job_store.active_execs.lock().unwrap().insert(2, active);
        deposited
    };
    let first_waiter = tokio::spawn(async move { first_deposited.notified().await });
    let second_waiter = tokio::spawn(async move { second_deposited.notified().await });
    tokio::task::yield_now().await;

    // Regression guard: this is the crucial behavior -- callers awaiting
    // these oneshots must see an immediate result, not hang forever and
    // let the parent IPC call time out at 30s.
    job_store.fail_all("control channel closed: decode error");

    for rx in [rx1, rx2, rx3] {
        match rx.await {
            Ok(JobResult::Error { message }) => {
                assert!(message.contains("control channel closed"));
            }
            other => panic!("expected JobResult::Error, got {other:?}"),
        }
    }
    assert!(
        snap_rx.await.is_ok(),
        "snapshot_ready waiter must be resolved"
    );
    assert!(job_store.active_execs.lock().unwrap().is_empty());
    assert!(job_store.jobs.lock().unwrap().is_empty());
    tokio::time::timeout(std::time::Duration::from_millis(100), first_waiter)
        .await
        .expect("first exec waiter must be notified")
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_millis(100), second_waiter)
        .await
        .expect("second exec waiter must be notified")
        .unwrap();
}

// -----------------------------------------------------------------------
// Job completion via oneshot (integration-unit)
// -----------------------------------------------------------------------

#[test]
fn job_oneshot_send_receive() {
    let (tx, rx) = oneshot::channel::<JobResult>();
    tx.send(JobResult::Exec {
        stdout: b"hello".to_vec(),
        stderr: vec![],
        exit_code: 0,
        truncated: false,
    })
    .unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(rx).unwrap();
    match result {
        JobResult::Exec {
            stdout, exit_code, ..
        } => {
            assert_eq!(stdout, b"hello");
            assert_eq!(exit_code, 0);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn job_oneshot_dropped_sender() {
    let (tx, rx) = oneshot::channel::<JobResult>();
    drop(tx); // Simulate client disconnect

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(rx);
    assert!(result.is_err()); // RecvError
}

#[tokio::test]
async fn quiescence_timeout_fires() {
    let job_store = Arc::new(JobStore::new());
    let (tx, mut _rx) = tokio::sync::mpsc::channel::<HostToGuest>(16);

    // 1. Never send SnapshotReady
    let start = std::time::Instant::now();
    let result = with_quiescence(
        &tx,
        &job_store,
        std::time::Duration::from_millis(100),
        || async { Ok(()) },
    )
    .await;

    let elapsed = start.elapsed();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("timed out"));
    assert!(elapsed.as_millis() >= 100);
}

#[tokio::test]
async fn quiescence_success_runs_operation() {
    let job_store = Arc::new(JobStore::new());
    let (tx, mut _rx) = tokio::sync::mpsc::channel::<HostToGuest>(16);

    // Simulate the guest sending SnapshotReady
    {
        let js = Arc::clone(&job_store);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            if let Some(sender) = js.snapshot_ready.lock().unwrap().take() {
                capsem_core::try_send!("test_snapshot_ready", sender.send(()));
            }
        });
    }

    let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let executed_clone = Arc::clone(&executed);
    let result = with_quiescence(&tx, &job_store, std::time::Duration::from_secs(5), || {
        let e = executed_clone;
        async move {
            e.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    })
    .await;

    assert!(result.is_ok());
    assert!(executed.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn quiescence_channel_closed_returns_error() {
    let job_store = Arc::new(JobStore::new());
    let (tx, mut _rx) = tokio::sync::mpsc::channel::<HostToGuest>(16);

    // Drop the sender without sending, simulating channel close
    {
        let js = Arc::clone(&job_store);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            // Take and drop the sender without sending
            let _ = js.snapshot_ready.lock().unwrap().take();
        });
    }

    let result = with_quiescence(
        &tx,
        &job_store,
        std::time::Duration::from_secs(5),
        || async { Ok(()) },
    )
    .await;

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("closed prematurely"));
}
