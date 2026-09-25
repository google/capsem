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
    super::super::exec_output::deposit(
        &js,
        2,
        super::super::exec_output::ExecCapture {
            stdout: b"independent".to_vec(),
            stdout_bytes: 11,
            ..Default::default()
        },
    )
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
    super::super::exec_output::deposit(
        &js,
        1,
        super::super::exec_output::ExecCapture {
            stdout: b"last bytes".to_vec(),
            stdout_bytes: 10,
            ..Default::default()
        },
    )
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

/// A captured exec whose output never reaches the host must not report
/// success with empty output. Under load the EXEC-port reader can miss the
/// deposit bound; the result used to be `Exec { stdout: [], exit_code: 0 }`,
/// indistinguishable from a command that printed nothing.
#[tokio::test(start_paused = true)]
async fn output_that_never_arrives_is_an_error_not_an_empty_success() {
    let js = Arc::new(JobStore::new());
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let rules = Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
    )));
    let plugins = Arc::new(std::sync::RwLock::new(Arc::new(std::collections::BTreeMap::new())));
    js.active_execs.lock().unwrap().insert(1, ActiveExec::new());
    let (tx, result) = oneshot::channel();
    js.jobs.lock().unwrap().insert(1, tx);

    // ExecDone arrives; the reader never deposits.
    super::super::handle_guest_msg(
        GuestToHost::ExecDone { id: 1, exit_code: 0 },
        &js,
        &db,
        &rules,
        &plugins,
    )
    .await;

    match result.await.unwrap() {
        JobResult::Error { message } => assert!(message.contains("output"), "the error names what was lost: {message}"),
        other => panic!("lost output reported as {other:?}"),
    }
}

/// A command that genuinely printed nothing still deposits (an empty capture
/// at EOF), and must stay a successful empty result.
#[tokio::test(start_paused = true)]
async fn a_deposited_empty_capture_stays_a_successful_empty_result() {
    let js = Arc::new(JobStore::new());
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let rules = Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
    )));
    let plugins = Arc::new(std::sync::RwLock::new(Arc::new(std::collections::BTreeMap::new())));
    js.active_execs.lock().unwrap().insert(1, ActiveExec::new());
    let (tx, result) = oneshot::channel();
    js.jobs.lock().unwrap().insert(1, tx);
    super::super::exec_output::deposit(&js, 1, super::super::exec_output::ExecCapture::default())
        .unwrap()
        .notify_one();

    super::super::handle_guest_msg(
        GuestToHost::ExecDone { id: 1, exit_code: 0 },
        &js,
        &db,
        &rules,
        &plugins,
    )
    .await;

    assert!(matches!(
        result.await.unwrap(),
        JobResult::Exec { stdout, exit_code: 0, truncated: false, .. } if stdout.is_empty()
    ));
}

mod ledger;
