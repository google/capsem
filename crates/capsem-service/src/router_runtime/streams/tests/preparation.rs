//! Attached startup shares policy admission only until its launch is dispatched.
use super::*;

#[tokio::test]
async fn attached_dispatch_waits_for_policy_mutation_but_does_not_lock_the_workload_lifetime() {
    let fx = fixture().await;
    fx.state.containers.stage_for_tests("box", "registry.example/app:1");
    let mutation = fx.state.policy_mutation.begin().await;
    let dispatched = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    let worker = owner(&fx.uds_path, {
        let dispatched = Arc::clone(&dispatched);
        let finish = Arc::clone(&finish);
        move |tx, rx| async move {
            let ServiceToProcess::ExecStream { id, .. } = rx.recv().await.unwrap() else {
                panic!("expected launch")
            };
            dispatched.notify_one();
            finish.notified().await;
            tx.send(ProcessToService::ExecResult {
                id,
                stdout: vec![],
                stderr: vec![],
                exit_code: 0,
                truncated: false,
            })
            .await
            .unwrap();
        }
    });
    let mut client = connect(fx.router.clone(), Some(stream::STREAM_SUBPROTOCOL))
        .await
        .unwrap();
    client
        .send(ClientMessage::Binary(
            encode_control(&StreamControl::Start {
                kind: StreamKind::Container,
                command: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            () = fx.state.policy_mutation.contended.notified() => {},
            () = dispatched.notified() => panic!("workload dispatched before serialized policy admission"),
        }
    })
    .await
    .unwrap();
    drop(mutation);
    tokio::time::timeout(std::time::Duration::from_secs(5), dispatched.notified())
        .await
        .unwrap();
    let mutation = tokio::time::timeout(std::time::Duration::from_secs(1), fx.state.policy_mutation.begin())
        .await
        .expect("the running workload must not retain the policy mutation lock");
    drop(mutation);
    assert_eq!(
        status(&next_server_frame(&mut client).await.unwrap()),
        StreamStatus::Started
    );
    finish.notify_one();
    assert_eq!(
        status(&next_server_frame(&mut client).await.unwrap()),
        StreamStatus::Exit {
            code: 0,
            truncated: false
        }
    );
    worker.await.unwrap();
}

#[tokio::test]
async fn attached_dispatch_refuses_an_unacknowledged_environment_before_exec() {
    let fx = fixture().await;
    fx.state.containers.stage_for_tests("box", "registry.example/app:1");
    fx.state.instances.lock().unwrap().get_mut("box").unwrap().environment =
        Some(crate::environment_launch::EnvironmentBinding::starting(
            capsem_logger::events::EnvironmentRevisionParent {
                environment_id: "00000000-0000-4000-8000-000000000001".into(),
                revision_id: "00000000-0000-4000-8000-000000000002".into(),
            },
            "unacknowledged".into(),
        ));
    let worker = owner(&fx.uds_path, |_, rx| async move {
        assert!(
            rx.recv().await.is_err(),
            "unacknowledged preparation must never reach ExecStream"
        );
    });
    let mut client = connect(fx.router.clone(), Some(stream::STREAM_SUBPROTOCOL))
        .await
        .unwrap();
    client
        .send(ClientMessage::Binary(
            encode_control(&StreamControl::Start {
                kind: StreamKind::Container,
                command: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    match status(&next_server_frame(&mut client).await.unwrap()) {
        StreamStatus::Error { message } => assert!(message.contains("not been acknowledged"), "{message}"),
        other => panic!("unacknowledged environment reached attached execution: {other:?}"),
    }
    worker.await.unwrap();
    assert_eq!(
        fx.state.containers.status("box").unwrap().state,
        capsem_api::ContainerState::Failed
    );
}
