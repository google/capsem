use super::*;
use capsem_proto::approval::{ApprovalClientMessage, ApprovalOutcome, ApprovalServerMessage};
use std::time::Duration;

fn configure(dispatch: &mut ExecDispatch, peer: &crate::tests::ApprovalPeer) {
    dispatch.approvals = Some(Arc::clone(&peer.capability));
    let profile = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.ask_exec]
        name = "ask_exec"
        action = "ask"
        match = 'process.command.contains("PRIVATE_EXEC_CANARY")'
    "#,
    )
    .unwrap();
    *dispatch.rules.write().unwrap() = Arc::new(
        capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
            &profile,
            capsem_core::net::policy_config::SecurityRuleSource::User,
        )
        .unwrap(),
    );
}

#[tokio::test]
async fn original_exec_waits_for_approval_and_only_approved_command_reaches_guest() {
    for outcome in [
        ApprovalOutcome::Approved,
        ApprovalOutcome::Denied,
        ApprovalOutcome::Expired,
    ] {
        let Bridge {
            _dir: _directory,
            mut dispatch,
            mut guest,
            ..
        } = bridge();
        let peer = crate::tests::ApprovalPeer::new().await;
        configure(&mut dispatch, &peer);
        let jobs = Arc::clone(&dispatch.jobs);
        jobs.active_execs
            .lock()
            .unwrap()
            .insert(7, crate::job_store::ActiveExec::new());
        let (reply, mut result) = tokio::sync::oneshot::channel();
        jobs.jobs.lock().unwrap().insert(7, reply);
        let command = "printf PRIVATE_EXEC_CANARY";
        let call = tokio::spawn(async move { dispatch.dispatch(7, command.into(), ExecTarget::Workload).await });
        let ApprovalClientMessage::Begin { request_id, request } =
            tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv())
                .await
                .expect("original exec is held for human approval")
                .unwrap()
        else {
            panic!("begin")
        };
        assert!(guest.try_recv().is_err(), "no guest execution before approval");
        assert!(!request.summary.contains("PRIVATE_EXEC_CANARY"));
        peer.sender
            .send(ApprovalServerMessage { request_id, outcome })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), call)
            .await
            .unwrap()
            .unwrap();
        if outcome == ApprovalOutcome::Approved {
            assert_eq!(
                dispatched(&mut guest),
                (7, guest_command(command.into(), ExecTarget::Workload))
            );
            assert!(
                result.try_recv().is_err(),
                "original job stays registered for guest completion"
            );
        } else {
            assert!(guest.try_recv().is_err());
            assert!(matches!(result.await.unwrap(), JobResult::Error { .. }));
            assert!(!jobs.active_execs.lock().unwrap().contains_key(&7));
        }
    }
}

#[tokio::test]
async fn runtime_end_cancels_held_exec_without_touching_a_reused_job_id() {
    let Bridge {
        _dir: _directory,
        mut dispatch,
        mut guest,
        ..
    } = bridge();
    let peer = crate::tests::ApprovalPeer::new().await;
    configure(&mut dispatch, &peer);
    let jobs = Arc::clone(&dispatch.jobs);
    jobs.active_execs
        .lock()
        .unwrap()
        .insert(7, crate::job_store::ActiveExec::new());
    let (reply, result) = tokio::sync::oneshot::channel();
    jobs.jobs.lock().unwrap().insert(7, reply);
    let call = tokio::spawn(async move {
        dispatch
            .dispatch(7, "printf PRIVATE_EXEC_CANARY".into(), ExecTarget::Vm)
            .await
    });
    let ApprovalClientMessage::Begin { request_id, .. } =
        tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("begin")
    };
    jobs.fail_all("runtime ended");
    assert!(matches!(result.await.unwrap(), JobResult::Error { .. }));
    jobs.active_execs
        .lock()
        .unwrap()
        .insert(7, crate::job_store::ActiveExec::new());
    let (new_reply, mut new_result) = tokio::sync::oneshot::channel();
    jobs.jobs.lock().unwrap().insert(7, new_reply);
    tokio::time::timeout(Duration::from_secs(2), call)
        .await
        .expect("runtime end cancels its original human wait")
        .unwrap();
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv()).await.unwrap().unwrap(), ApprovalClientMessage::Cancel { request_id: id } if id == request_id)
    );
    peer.sender
        .send(ApprovalServerMessage {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    assert!(guest.try_recv().is_err(), "stale approval must not execute anything");
    assert!(jobs.active_execs.lock().unwrap().contains_key(&7));
    assert!(
        new_result.try_recv().is_err(),
        "old completion cannot consume a replacement job"
    );
}

#[tokio::test]
async fn held_exec_keeps_an_independent_command_dispatchable() {
    let Bridge {
        _dir: _directory,
        mut dispatch,
        mut guest,
        ..
    } = bridge();
    let peer = crate::tests::ApprovalPeer::new().await;
    configure(&mut dispatch, &peer);
    let mut calls = tokio::task::JoinSet::new();
    for id in [7, 8] {
        dispatch
            .jobs
            .active_execs
            .lock()
            .unwrap()
            .insert(id, crate::job_store::ActiveExec::new());
    }
    dispatch.spawn(&mut calls, 7, "printf PRIVATE_EXEC_CANARY".into(), ExecTarget::Vm);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv())
            .await
            .unwrap()
            .unwrap(),
        ApprovalClientMessage::Begin { .. }
    ));
    dispatch.spawn(&mut calls, 8, "printf independent".into(), ExecTarget::Vm);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), guest.recv())
            .await
            .unwrap()
            .unwrap(),
        HostToGuest::Exec { id: 8, .. }
    ));
    dispatch.jobs.cancel_exec(7);
    tokio::time::timeout(Duration::from_secs(2), async {
        while calls.join_next().await.is_some() {}
    })
    .await
    .unwrap();
    assert!(guest.try_recv().is_err());
}

#[tokio::test]
async fn owned_exec_waiters_refuse_overload_without_blocking_the_control_reader() {
    let Bridge {
        _dir: _directory,
        mut dispatch,
        mut guest,
        ..
    } = bridge();
    let peer = crate::tests::ApprovalPeer::new().await;
    configure(&mut dispatch, &peer);
    let mut calls = tokio::task::JoinSet::new();
    let mut results = Vec::new();
    for id in 1..=(capsem_proto::APPROVAL_PENDING_PER_SESSION as u64 + 1) {
        dispatch
            .jobs
            .active_execs
            .lock()
            .unwrap()
            .insert(id, crate::job_store::ActiveExec::new());
        let (reply, result) = tokio::sync::oneshot::channel();
        dispatch.jobs.jobs.lock().unwrap().insert(id, reply);
        results.push(result);
        dispatch.spawn(&mut calls, id, "printf PRIVATE_EXEC_CANARY".into(), ExecTarget::Vm);
    }
    assert_eq!(calls.len(), capsem_proto::APPROVAL_PENDING_PER_SESSION);
    assert!(matches!(results.pop().unwrap().await.unwrap(), JobResult::Error { .. }));
    for _ in 0..capsem_proto::APPROVAL_PENDING_PER_SESSION {
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv())
                .await
                .unwrap()
                .unwrap(),
            ApprovalClientMessage::Begin { .. }
        ));
    }
    dispatch.jobs.fail_all("control owner ended");
    tokio::time::timeout(Duration::from_secs(2), async {
        while calls.join_next().await.is_some() {}
    })
    .await
    .unwrap();
    assert!(guest.try_recv().is_err());
}

#[tokio::test]
async fn pending_exec_rechecks_its_policy_before_guest_delivery() {
    let Bridge {
        _dir: _directory,
        mut dispatch,
        mut guest,
        ..
    } = bridge();
    let peer = crate::tests::ApprovalPeer::new().await;
    configure(&mut dispatch, &peer);
    let rules = Arc::clone(&dispatch.rules);
    let jobs = Arc::clone(&dispatch.jobs);
    jobs.active_execs
        .lock()
        .unwrap()
        .insert(7, crate::job_store::ActiveExec::new());
    let (reply, result) = tokio::sync::oneshot::channel();
    jobs.jobs.lock().unwrap().insert(7, reply);
    let call = tokio::spawn(async move {
        dispatch
            .dispatch(7, "printf PRIVATE_EXEC_CANARY".into(), ExecTarget::Vm)
            .await
    });
    let ApprovalClientMessage::Begin { request_id, .. } =
        tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("begin")
    };
    *rules.write().unwrap() = Arc::new(capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()));
    peer.sender
        .send(ApprovalServerMessage {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    call.await.unwrap();
    assert!(matches!(result.await.unwrap(), JobResult::Error { .. }));
    assert!(guest.try_recv().is_err());
}

#[tokio::test]
async fn cancellation_does_not_wait_for_a_full_guest_control_queue() {
    let Bridge {
        _dir: _directory,
        mut dispatch,
        mut guest,
        ..
    } = bridge();
    let peer = crate::tests::ApprovalPeer::new().await;
    configure(&mut dispatch, &peer);
    let jobs = Arc::clone(&dispatch.jobs);
    jobs.active_execs
        .lock()
        .unwrap()
        .insert(7, crate::job_store::ActiveExec::new());
    let (reply, result) = tokio::sync::oneshot::channel();
    jobs.jobs.lock().unwrap().insert(7, reply);
    for _ in 0..4 {
        dispatch.hub.send(HostToGuest::Ping { epoch_secs: 1 }).await.unwrap();
    }
    let call = tokio::spawn(async move {
        dispatch
            .dispatch(7, "printf PRIVATE_EXEC_CANARY".into(), ExecTarget::Vm)
            .await
    });
    let ApprovalClientMessage::Begin { request_id, .. } =
        tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("begin")
    };
    peer.sender
        .send(ApprovalServerMessage {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    jobs.cancel_exec(7);
    tokio::time::timeout(Duration::from_secs(2), call)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.await.unwrap(), JobResult::Error { .. }));
    for _ in 0..4 {
        assert!(matches!(guest.try_recv().unwrap(), HostToGuest::Ping { .. }));
    }
    assert!(guest.try_recv().is_err());
}

#[tokio::test]
async fn oversized_original_command_is_refused_before_spawning_or_human_wait() {
    let Bridge {
        _dir: _directory,
        mut dispatch,
        mut guest,
        ..
    } = bridge();
    let peer = crate::tests::ApprovalPeer::new().await;
    configure(&mut dispatch, &peer);
    dispatch
        .jobs
        .active_execs
        .lock()
        .unwrap()
        .insert(7, crate::job_store::ActiveExec::new());
    let (reply, result) = tokio::sync::oneshot::channel();
    dispatch.jobs.jobs.lock().unwrap().insert(7, reply);
    let mut calls = tokio::task::JoinSet::new();
    let command = "x".repeat(capsem_proto::APPROVAL_HELD_BYTES_PER_REQUEST / 4 + 1);
    dispatch.spawn(&mut calls, 7, command, ExecTarget::Vm);
    assert!(calls.is_empty());
    assert!(matches!(result.await.unwrap(), JobResult::Error { .. }));
    assert!(guest.try_recv().is_err());
    assert!(tokio::time::timeout(Duration::from_millis(50), peer.receiver.recv())
        .await
        .is_err());
}
