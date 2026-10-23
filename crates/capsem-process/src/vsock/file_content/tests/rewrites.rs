use super::*;
use capsem_core::net::policy_config::{DetectionLevel, SecurityPluginConfig, SecurityPluginMode};

async fn export(data: Vec<u8>, plugin: &str) -> JobResult {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&temp.path().join("session.db"), 64).unwrap());
    let jobs = Arc::new(JobStore::new());
    let state = Arc::new(FileReadState::default());
    let (sender, result) = tokio::sync::oneshot::channel();
    jobs.jobs.lock().unwrap().insert(1, sender);
    jobs.active_file_ops.lock().unwrap().insert(
        1,
        ActiveFileOp::Read {
            path: "/workspace/export.txt".into(),
            state,
        },
    );
    let rules = Arc::new(RwLock::new(Arc::new(SecurityRuleSet::new(Vec::new()))));
    let plugins = Arc::new(RwLock::new(Arc::new(std::collections::BTreeMap::from([(
        plugin.into(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Rewrite,
            detection_level: DetectionLevel::High,
        },
    )]))));
    let mut calls = tokio::task::JoinSet::new();
    crate::vsock::handle_guest_msg_with_tasks(
        capsem_proto::GuestToHost::FileContent {
            id: 1,
            path: "untrusted".into(),
            data,
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
    result.await.unwrap()
}

#[tokio::test]
async fn guest_export_applies_complete_utf8_rewrite_and_refuses_incomplete_mutation() {
    let result = export("αEICAR🦊".as_bytes().to_vec(), "dummy_pre_eicar").await;
    assert!(
        matches!(result, JobResult::ReadFile { data: Some(ref data), error: None }
        if data == "αCAPSEM_REWRITTEN_EICAR🦊".as_bytes())
    );
    let mut oversized = b"EICAR".to_vec();
    oversized.resize(crate::ipc::file_boundary::FILE_SECURITY_CONTENT_PREVIEW_MAX + 1, b'x');
    for data in [oversized, b"EICAR\xff".to_vec()] {
        assert!(matches!(
            export(data, "dummy_pre_eicar").await,
            JobResult::ReadFile {
                data: None,
                error: Some(_)
            }
        ));
    }
}

#[tokio::test]
async fn logging_plugin_configuration_preserves_large_original_bytes() {
    let mut data = b"Bearer sk-test-forensic-only".to_vec();
    data.resize(crate::ipc::file_boundary::FILE_SECURITY_CONTENT_PREVIEW_MAX + 1, b'x');
    let result = export(data.clone(), "log_sanitizer").await;
    assert!(matches!(result, JobResult::ReadFile { data: Some(actual), error: None } if actual == data));
}
