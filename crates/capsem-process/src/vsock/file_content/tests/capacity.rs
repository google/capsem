use super::*;

#[tokio::test]
async fn bounded_file_memory_preserves_an_uncontended_ten_mib_transfer() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&temp.path().join("session.db"), 64).unwrap());
    let jobs = Arc::new(JobStore::new());
    let state = Arc::new(FileReadState::default());
    let (sender, result) = tokio::sync::oneshot::channel();
    jobs.jobs.lock().unwrap().insert(1, sender);
    jobs.active_file_ops.lock().unwrap().insert(
        1,
        ActiveFileOp::Read {
            path: "/workspace/large.bin".into(),
            state,
        },
    );
    let rules = Arc::new(RwLock::new(Arc::new(SecurityRuleSet::new(Vec::new()))));
    let plugins = Arc::new(RwLock::new(Arc::new(Default::default())));
    let mut calls = tokio::task::JoinSet::new();
    crate::vsock::handle_guest_msg_with_tasks(
        capsem_proto::GuestToHost::FileContent {
            id: 1,
            path: "untrusted".into(),
            data: vec![b'Z'; 10 * 1024 * 1024],
        },
        &jobs,
        &db,
        &rules,
        &plugins,
        &mut calls,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while calls.join_next().await.is_some() {}
    })
    .await
    .unwrap();
    let JobResult::ReadFile {
        data: Some(data),
        error: None,
    } = result.await.unwrap()
    else {
        panic!("complete original file")
    };
    assert_eq!(data.len(), 10 * 1024 * 1024);
    assert!(data.iter().all(|byte| *byte == b'Z'));
    assert_eq!(
        jobs.file_memory.available_permits(),
        capsem_proto::APPROVAL_HELD_BYTES_TOTAL
    );
}
