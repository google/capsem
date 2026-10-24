use super::*;

#[tokio::test]
async fn completed_guest_write_keeps_facts_without_creating_a_fake_pending_approval() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let db = Arc::new(capsem_logger::DbWriter::open(&path, 16).unwrap());
    let profile = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.observe_completed_write]
        name = "observe_completed_write"
        action = "ask"
        match = 'file.write.path == "/workspace/already-written.txt"'
    "#,
    )
    .unwrap();
    let rules = capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .unwrap();
    let rules = Arc::new(std::sync::RwLock::new(Arc::new(rules)));
    let jobs = Arc::new(JobStore::new());
    jobs.active_file_ops.lock().unwrap().insert(
        7,
        ActiveFileOp::Write {
            observation: Arc::new(crate::job_store::WriteObservation {
                path: "/workspace/already-written.txt".into(),
                size: 15,
                content: "already written".into(),
            }),
        },
    );
    let (reply, result) = tokio::sync::oneshot::channel();
    jobs.jobs.lock().unwrap().insert(7, reply);
    handle_guest_msg(
        GuestToHost::FileOpDone { id: 7 },
        &jobs,
        &db,
        &rules,
        &empty_plugin_policy(),
    )
    .await;
    assert!(matches!(
        result.await.unwrap(),
        JobResult::WriteFile { success: true, .. }
    ));
    db.flush_checked().await.unwrap();
    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let pending: serde_json::Value =
        serde_json::from_str(&reader.query_raw("SELECT COUNT(*) FROM security_ask_events").unwrap()).unwrap();
    assert_eq!(
        pending["rows"][0][0], 0,
        "a completed write has no original operation left to approve"
    );
    let facts: serde_json::Value =
        serde_json::from_str(&reader.query_raw("SELECT action, path, size FROM fs_events").unwrap()).unwrap();
    assert_eq!(
        facts["rows"][0],
        serde_json::json!(["modified", "/workspace/already-written.txt", 15])
    );
    let matched: serde_json::Value = serde_json::from_str(
        &reader
            .query_raw("SELECT rule_action FROM security_rule_events")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(matched["rows"][0][0], "ask", "observational rule evidence is retained");
}
