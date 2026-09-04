use super::*;
use crate::events::{Decision, ProfileMutationEvent, ProfileMutationStatus};
use crate::WriteOp;

#[tokio::test]
async fn read_cache_domains_ignore_orthogonal_profile_mutation_rows() {
    let path = super::handle_tests::temp_db_path("read-cache-domains");
    let db = DbHandle::open(&path).expect("open handle");
    let all_before = db.read_cache_epoch(ReadCacheDomain::All);
    let sessions_before = db.read_cache_epoch(ReadCacheDomain::SessionSummary);

    db.write(WriteOp::ProfileMutationEvent(ProfileMutationEvent {
        timestamp_unix_ms: 1_789_000_000_000,
        mutation_id: "a1b2c3d4e5f6".into(),
        profile_id: "code".into(),
        actor: "test".into(),
        category: "mcp".into(),
        filename: "enforcement.toml".into(),
        affected_path: "profiles/code/enforcement.toml".into(),
        target_kind: "mcp_default".into(),
        target_key: "default.mcp".into(),
        operation: "permission".into(),
        rule_id: Some("default.mcp".into()),
        old_hash: format!("blake3:{}", "1".repeat(64)),
        old_size: 10,
        new_hash: format!("blake3:{}", "2".repeat(64)),
        new_size: 20,
        status: ProfileMutationStatus::Applied,
        error: None,
        trace_id: None,
    }))
    .await
    .expect("write profile mutation");

    assert!(db.read_cache_epoch(ReadCacheDomain::All) > all_before);
    assert_eq!(
        db.read_cache_epoch(ReadCacheDomain::SessionSummary),
        sessions_before,
        "profile mutation rows cannot change the session summary projection"
    );

    db.write(WriteOp::NetEvent(super::handle_tests::make_net_event(
        "session-summary.example",
        Decision::Allowed,
    )))
    .await
    .expect("write session event");
    assert!(db.read_cache_epoch(ReadCacheDomain::SessionSummary) > sessions_before);
}
