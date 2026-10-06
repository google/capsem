use super::*;

#[tokio::test]
async fn bound_shutdown_releases_vz_guard_before_waiting_for_its_child_reaper() {
    let state = crate::tests::make_test_state();
    let id = "bound-retirement";
    let session = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(&session).unwrap();
    let socket = state.instance_socket_path(id).unwrap();
    let child = tokio::process::Command::new("sh")
        .args(["-c", "exec sleep 30"])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    insert_fake_instance_with_session_dir(&state, id, child.id().unwrap(), session.clone());
    let generation = state.instances.lock().unwrap().get(id).unwrap().generation;
    let retirement = state.retirements.register(id, generation).unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        id.into(),
        id.into(),
        Arc::clone(&state),
        socket,
        session,
        retirement,
    );
    let workload = state.containers.begin(id, "held-before-shutdown");
    let lease = state.containers.work_lease(id, workload).unwrap();
    let shutdown = {
        let state = Arc::clone(&state);
        tokio::spawn(async move { shutdown_vm_process(&state, id, ShutdownMode::Discard, Some(generation)).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state.instances.lock().unwrap().contains_key(id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let entered = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let writer = {
        let state = Arc::clone(&state);
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        tokio::spawn(async move {
            let _guard = state.lifecycle.vz.write().await;
            entered.add_permits(1);
            release.acquire().await.unwrap().forget();
        })
    };
    tokio::task::yield_now().await;
    drop(lease);
    tokio::time::timeout(std::time::Duration::from_secs(2), entered.acquire())
        .await
        .expect("shutdown retained VZ read ownership while joining its reaper")
        .unwrap()
        .forget();
    assert!(
        !shutdown.is_finished(),
        "the original reaper must still retire behind the queued writer"
    );
    release.add_permits(1);
    writer.await.unwrap();
    reaper.await.unwrap();
    assert!(shutdown.await.unwrap().unwrap().is_some());
}

#[tokio::test]
async fn replaced_aborted_setup_cannot_escape_its_blocking_retirement_barrier() {
    let owner = Arc::new(ContainerSetups::default());
    let old = owner.begin("same-id", "old-image");
    let async_lease = owner.work_lease("same-id", old).unwrap();
    let writer_lease = owner.work_lease("same-id", old).unwrap();
    let entered = Arc::new(tokio::sync::Semaphore::new(0));
    let (release, wait) = std::sync::mpsc::channel();
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("last-write");
    let waiter = {
        let entered = Arc::clone(&entered);
        let output = output.clone();
        tokio::spawn(async move {
            let _lease = async_lease;
            tokio::task::spawn_blocking(move || {
                let _lease = writer_lease;
                entered.add_permits(1);
                wait.recv().unwrap();
                std::fs::write(output, b"old worker finished").unwrap();
            })
            .await
            .unwrap();
        })
    };
    owner.attach_task("same-id", old, waiter.abort_handle());
    entered.acquire().await.unwrap().forget();
    let replacement = owner.begin("same-id", "new-image");
    assert_ne!(old, replacement);
    assert!(owner.work_lease("same-id", old).is_none());
    assert_eq!(owner.status("same-id").unwrap().image, "new-image");
    assert!(waiter.await.unwrap_err().is_cancelled());
    let drain = {
        let owner = Arc::clone(&owner);
        tokio::spawn(async move {
            owner.cancel_and_wait("same-id").await;
        })
    };
    tokio::task::yield_now().await;
    assert!(!drain.is_finished());
    release.send(()).unwrap();
    drain.await.unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"old worker finished");
    assert!(
        owner.retiring.lock().unwrap().is_empty(),
        "idle generations must release retirement tracking"
    );
}

#[tokio::test]
async fn cancelled_workload_drains_its_blocking_writer_before_retirement() {
    let owner = Arc::new(ContainerSetups::default());
    let generation = owner.begin("original", "image");
    let lease = owner.work_lease("original", generation).unwrap();
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("writer-finished");
    let (release, wait) = std::sync::mpsc::channel();
    let entered = Arc::new(tokio::sync::Semaphore::new(0));
    let worker = {
        let entered = Arc::clone(&entered);
        let output = output.clone();
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            entered.add_permits(1);
            wait.recv().unwrap();
            std::fs::write(output, b"finished before retirement").unwrap();
        })
    };
    entered.acquire().await.unwrap().forget();
    let drain = {
        let owner = Arc::clone(&owner);
        tokio::spawn(async move {
            owner.cancel_and_wait("original").await;
        })
    };
    tokio::task::yield_now().await;
    assert!(owner.status("original").is_none());
    assert!(owner.work_lease("original", generation).is_none());
    assert!(!drain.is_finished());
    let unrelated = owner.begin("unrelated", "image");
    assert!(owner.work_lease("unrelated", unrelated).is_some());
    owner.cancel_and_wait("other").await;
    assert!(!drain.is_finished());
    release.send(()).unwrap();
    worker.await.unwrap();
    drain.await.unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"finished before retirement");
    owner.cancel_and_wait("unrelated").await;
}
