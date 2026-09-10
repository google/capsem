use super::*;

#[test]
fn running_snapshot_responses_require_known_origins() {
    let mut status = capsem_proto::ipc::SnapshotStatus {
        total: 1,
        auto_count: 0,
        manual_count: 1,
        manual_available: 11,
        snapshots: vec![capsem_proto::ipc::SnapshotSlotStatus {
            checkpoint: "cp-10".into(),
            slot: 10,
            origin: "manual".into(),
            name: Some("baseline".into()),
            timestamp: "2026-09-10T00:00:00Z".into(),
            hash: Some("abc".into()),
        }],
    };
    let result = snapshot_response(status.clone()).unwrap();
    assert_eq!(result.snapshots[0].origin, api::SnapshotOrigin::Manual);
    assert_eq!(result.manual_available, 11);
    status.snapshots[0].origin = "future_unknown".into();
    let error = snapshot_response(status).unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_GATEWAY);
    assert!(error.1.contains("future_unknown"));
}
