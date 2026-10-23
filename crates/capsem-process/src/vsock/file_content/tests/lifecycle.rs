use super::*;

struct Export {
    _temp: tempfile::TempDir,
    peer: crate::tests::ApprovalPeer,
    jobs: Arc<JobStore>,
    state: Arc<FileReadState>,
    worker: tokio::task::JoinHandle<()>,
    result: tokio::sync::oneshot::Receiver<JobResult>,
}

impl Export {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(DbWriter::open(&temp.path().join("session.db"), 64).unwrap());
        let peer = crate::tests::ApprovalPeer::new().await;
        let profile = SecurityRuleProfile::parse_toml(
            r#"
            [profiles.rules.ask_export]
            name = "ask_export"
            action = "ask"
            match = 'file.export.path == "/workspace/private.txt"'
        "#,
        )
        .unwrap();
        let rules = Arc::new(RwLock::new(Arc::new(
            SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap(),
        )));
        let plugins = Arc::new(RwLock::new(Arc::new(Default::default())));
        let jobs = Arc::new(JobStore::new());
        let state = Arc::new(FileReadState {
            approvals: Some(Arc::clone(&peer.capability)),
            ..Default::default()
        });
        *state.arrived.lock().unwrap() = true;
        state.received.notify_one();
        let (sender, result) = tokio::sync::oneshot::channel();
        jobs.jobs.lock().unwrap().insert(7, sender);
        jobs.active_file_ops.lock().unwrap().insert(
            7,
            ActiveFileOp::Read {
                path: "/workspace/private.txt".into(),
                state: Arc::clone(&state),
            },
        );
        let worker = tokio::spawn(complete(
            7,
            ActiveFileOp::Read {
                path: "/workspace/private.txt".into(),
                state: Arc::clone(&state),
            },
            b"private export data".to_vec(),
            Arc::clone(&jobs),
            db,
            rules,
            plugins,
        ));
        Self {
            _temp: temp,
            peer,
            jobs,
            state,
            worker,
            result,
        }
    }

    async fn begin(&self) -> u64 {
        let ApprovalClientMessage::Begin { request_id, .. } =
            tokio::time::timeout(Duration::from_secs(1), self.peer.receiver.recv())
                .await
                .unwrap()
                .unwrap()
        else {
            panic!("begin")
        };
        request_id
    }
}

#[tokio::test]
async fn stale_export_cannot_deliver_after_its_job_identity_is_reused() {
    let export = Export::new().await;
    let request_id = export.begin().await;
    let new_state = Arc::new(FileReadState::default());
    let (new_sender, mut new_result) = tokio::sync::oneshot::channel();
    export.jobs.jobs.lock().unwrap().insert(7, new_sender);
    export.jobs.active_file_ops.lock().unwrap().insert(
        7,
        ActiveFileOp::Read {
            path: "/workspace/new.txt".into(),
            state: Arc::clone(&new_state),
        },
    );
    export
        .peer
        .sender
        .send(ApprovalServerMessage {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), export.worker)
        .await
        .unwrap()
        .unwrap();
    assert!(
        export.result.await.is_err(),
        "a stale callback cannot release its held bytes"
    );
    assert!(matches!(
        new_result.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert!(export.jobs.jobs.lock().unwrap().contains_key(&7));
    assert!(!new_state.cancellation.is_cancelled());
    assert!(!export.state.cancellation.is_cancelled());
}

#[tokio::test]
async fn runtime_end_cancels_the_original_export_without_delivering_bytes() {
    let export = Export::new().await;
    let request_id = export.begin().await;
    export.jobs.fail_all("runtime ended");
    tokio::time::timeout(Duration::from_secs(1), export.worker)
        .await
        .unwrap()
        .unwrap();
    assert!(export.result.await.is_err());
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(1), export.peer.receiver.recv()).await.unwrap().unwrap(),
        ApprovalClientMessage::Cancel { request_id: id } if id == request_id)
    );
    assert!(export.jobs.active_file_ops.lock().unwrap().is_empty());
}

#[tokio::test]
async fn disconnected_export_cancels_the_original_wait() {
    let export = Export::new().await;
    let request_id = export.begin().await;
    drop(export.result);
    tokio::time::timeout(Duration::from_secs(1), export.worker)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(1), export.peer.receiver.recv()).await.unwrap().unwrap(),
        ApprovalClientMessage::Cancel { request_id: id } if id == request_id)
    );
    assert!(export.jobs.active_file_ops.lock().unwrap().is_empty());
}

#[tokio::test]
async fn denied_or_expired_export_never_delivers_the_held_bytes() {
    for outcome in [ApprovalOutcome::Denied, ApprovalOutcome::Expired] {
        let export = Export::new().await;
        let request_id = export.begin().await;
        export
            .peer
            .sender
            .send(ApprovalServerMessage { request_id, outcome })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), export.worker)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            export.result.await.unwrap(),
            JobResult::ReadFile {
                data: None,
                error: Some(_)
            }
        ));
        assert!(export.jobs.active_file_ops.lock().unwrap().is_empty());
    }
}
