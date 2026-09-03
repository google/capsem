//! Snapshot listing tools paginate guest-supplied offsets; those are clamped
//! and combined without overflow.

use super::*;

fn populated() -> (tempfile::TempDir, PathBuf, AutoSnapshotScheduler) {
    let (tmp, session, mut sched) = setup();
    sched.take_snapshot().unwrap();
    for i in 0..50 {
        std::fs::write(session.join(format!("workspace/file_{i:03}.txt")), "x").unwrap();
    }
    sched.take_snapshot().unwrap();
    (tmp, session, sched)
}

const HOSTILE_SIZES: &[u64] = &[u64::MAX, u64::MAX - 1, 1 << 40];

#[test]
fn snapshots_changes_survives_hostile_pagination_params() {
    let (_tmp, session, sched) = populated();
    let ws = session.join("workspace");
    for &value in HOSTILE_SIZES {
        let args = serde_json::json!({"start_index": 1, "max_length": value});
        let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
        assert!(resp.error.is_none(), "max_length={value}: {:?}", resp.error);
        assert!(extract_text(&resp).contains("file_049"));

        let args = serde_json::json!({"start_index": value, "max_length": value});
        let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
        assert!(resp.error.is_none(), "start_index={value}: {:?}", resp.error);
        let text = extract_text(&resp);
        assert!(text.contains("Content length:"), "{text}");
        assert!(
            !text.contains("to continue"),
            "past the end there is nothing to continue: {text}"
        );
    }
}

#[test]
fn snapshots_list_survives_hostile_pagination_params() {
    let (_tmp, session, sched) = populated();
    let ws = session.join("workspace");
    for &value in HOSTILE_SIZES {
        let args = serde_json::json!({"start_index": 1, "max_length": value});
        let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
        assert!(resp.error.is_none(), "max_length={value}: {:?}", resp.error);
        assert!(extract_text(&resp).contains("cp-1"));

        let args = serde_json::json!({"start_index": value, "max_length": 5});
        let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
        assert!(resp.error.is_none(), "start_index={value}: {:?}", resp.error);
    }
}
