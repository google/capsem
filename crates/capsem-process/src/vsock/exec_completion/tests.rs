use super::*;
use crate::job_store::ActiveExec;
use capsem_proto::GuestToHost;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

#[tokio::test]
async fn slow_stream_completion_leaves_control_and_other_jobs_responsive() {
    let js = Arc::new(JobStore::new());
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let rules = Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
    )));
    let plugins = Arc::new(std::sync::RwLock::new(Arc::new(std::collections::BTreeMap::new())));
    let (output, _consumer) = mpsc::channel(1);
    let mut active = ActiveExec::new();
    active.stream = Some(output);
    js.active_execs.lock().unwrap().insert(1, active);
    let (tx, mut result) = oneshot::channel();
    js.jobs.lock().unwrap().insert(1, tx);
    // ExecDone can arrive while the data reader waits for a slow host consumer.
    let dispatch = tokio::time::timeout(
        Duration::from_millis(250),
        super::super::handle_guest_msg(
            GuestToHost::ExecDone { id: 1, exit_code: 7 },
            &js,
            &db,
            &rules,
            &plugins,
        ),
    )
    .await;
    assert!(
        dispatch.is_ok(),
        "output backpressure must not occupy the VM control loop"
    );
    assert!(matches!(result.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    super::super::handle_guest_msg(GuestToHost::ShutdownComplete, &js, &db, &rules, &plugins).await;
    assert!(js.wait_shutdown_complete(Duration::from_millis(100)).await);

    let (other_tx, other_result) = oneshot::channel();
    js.jobs.lock().unwrap().insert(2, other_tx);
    js.active_execs.lock().unwrap().insert(2, ActiveExec::new());
    super::super::deposit_exec_output(&js, 2, b"independent".to_vec(), 11)
        .unwrap()
        .notify_one();
    super::super::handle_guest_msg(
        GuestToHost::ExecDone { id: 2, exit_code: 0 },
        &js,
        &db,
        &rules,
        &plugins,
    )
    .await;
    assert!(matches!(other_result.await.unwrap(), JobResult::Exec { stdout, .. } if stdout == b"independent"));

    // Replayed completion must not create a second waiter stealing the deposit.
    super::super::handle_guest_msg(
        GuestToHost::ExecDone { id: 1, exit_code: 0 },
        &js,
        &db,
        &rules,
        &plugins,
    )
    .await;
    super::super::deposit_exec_output(&js, 1, b"last bytes".to_vec(), 10)
        .unwrap()
        .notify_one();
    match tokio::time::timeout(Duration::from_secs(1), result)
        .await
        .unwrap()
        .unwrap()
    {
        JobResult::Exec {
            stdout,
            exit_code,
            truncated,
            ..
        } => {
            assert_eq!(exit_code, 7);
            assert!(
                stdout.is_empty(),
                "stream result must not replay already-delivered bytes"
            );
            assert!(!truncated);
        }
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(js.active_execs.lock().unwrap().is_empty());
}
