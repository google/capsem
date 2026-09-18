//! Failed-session housekeeping: preserving crash evidence and culling it.
//!
//! Moved out of `assets_registry.rs`, which had grown to hold the registry,
//! the instance table and this; the retention rule these now cover belongs
//! beside the cap rather than 700 lines below an unrelated fixture.

use super::*;
use crate::session_housekeeping::{retention_days_from_resolved, DEFAULT_RETENTION_DAYS};

// -----------------------------------------------------------------------
// preserve_failed_session_dir + cull_failed_sessions
//
// The post-mortem pipeline: when any of the three loss paths
// (wait_for_vm_ready timeout, dead-process cleanup, unexpected
// child exit) would have silently `remove_dir_all`'d a session dir,
// it's renamed to a `-failed-*` sibling instead so process.log,
// mcp-aggregator.stderr.log, serial.log, and session.db survive.
// Two rules bound what is kept: at most MAX_FAILED_SESSIONS of them, and
// none older than `vm.resources.retention_days`.
//
// The tests below pass the retention period explicitly. Reading it from the
// settings would make them depend on the settings file of whoever is running
// them, which is a test that passes or fails for reasons outside the tree.

#[test]
fn preserve_renames_session_dir_and_keeps_logs() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let session_dir = state.run_dir.join("sessions").join("vm-abc");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("process.log"), b"boot failed: ...").unwrap();
    std::fs::write(session_dir.join("serial.log"), b"kernel panic").unwrap();

    state
        .preserve_failed_session_dir(&session_dir, "vm-abc")
        .expect("session evidence should be preserved");

    assert!(!session_dir.exists(), "original dir should have been renamed");
    let entries: Vec<_> = std::fs::read_dir(state.run_dir.join("sessions"))
        .unwrap()
        .flatten()
        .collect();
    let failed = entries
        .iter()
        .find(|e| e.file_name().to_string_lossy().starts_with("vm-abc-failed-"))
        .expect("a vm-abc-failed-* dir must exist");
    let preserved = failed.path().join("process.log");
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&preserved, usize::MAX)
            .unwrap()
            .into_bytes(),
        b"boot failed: ..."
    );
    let preserved_serial = failed.path().join("serial.log");
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&preserved_serial, usize::MAX)
            .unwrap()
            .into_bytes(),
        b"kernel panic"
    );
}

#[test]
fn cull_keeps_newest_and_prunes_oldest() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    // Create MAX_FAILED_SESSIONS + 2 failed dirs with staggered mtimes.
    // Using filetime to set mtime lets us assert deterministically
    // which ones get pruned (oldest) vs kept (newest).
    let total = MAX_FAILED_SESSIONS + 2;
    for i in 0..total {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        let p = sessions.join(&name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("process.log"), format!("run {i}")).unwrap();
        // Older i -> older mtime, all of them well inside the retention
        // period so this test still measures only the count cap.
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs((total - i) as u64 * 10);
        filetime::set_file_mtime(&p, filetime::FileTime::from_system_time(when)).unwrap();
    }

    state.cull_failed_sessions_older_than(DEFAULT_RETENTION_DAYS).unwrap();

    let remaining: std::collections::HashSet<String> = std::fs::read_dir(&sessions)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        remaining.len(),
        MAX_FAILED_SESSIONS,
        "should keep exactly MAX_FAILED_SESSIONS, got {remaining:?}"
    );
    // Oldest two (i=0, i=1) must be pruned; newest MAX_FAILED_SESSIONS kept.
    for i in 0..2 {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        assert!(!remaining.contains(&name), "oldest dir {name} should have been culled");
    }
    for i in 2..total {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        assert!(remaining.contains(&name), "newer dir {name} should have been kept");
    }
}

#[test]
fn cull_is_noop_when_under_cap() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    for i in 0..3 {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        std::fs::create_dir_all(sessions.join(&name)).unwrap();
    }

    state.cull_failed_sessions_older_than(DEFAULT_RETENTION_DAYS).unwrap();

    assert_eq!(std::fs::read_dir(&sessions).unwrap().count(), 3);
}

#[test]
fn cull_ignores_non_failed_dirs() {
    // Running sessions (no `-failed-` in the name) must never be
    // culled. This is the safety property: a misnamed cull is a
    // production outage.
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    std::fs::create_dir_all(sessions.join("vm-alive")).unwrap();
    for i in 0..(MAX_FAILED_SESSIONS + 3) {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        std::fs::create_dir_all(sessions.join(&name)).unwrap();
    }

    state.cull_failed_sessions_older_than(DEFAULT_RETENTION_DAYS).unwrap();

    assert!(sessions.join("vm-alive").exists(), "active VM dir must not be culled");
}

#[test]
fn cull_removes_a_dir_past_the_retention_period_even_under_the_cap() {
    // The gap the count cap left: a machine that fails rarely keeps every
    // post-mortem forever, because nothing is ever above 32. Evidence about a
    // user's work has an expiry date whether or not disk is under pressure.
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    let stale = sessions.join("vm-old-failed-20260101-000000-aaaa");
    let fresh = sessions.join("vm-new-failed-20260101-000001-bbbb");
    for path in [&stale, &fresh] {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("process.log"), b"crash").unwrap();
    }
    let forty_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(40 * 86_400);
    filetime::set_file_mtime(&stale, filetime::FileTime::from_system_time(forty_days_ago)).unwrap();

    let culled = state
        .cull_failed_sessions_older_than(DEFAULT_RETENTION_DAYS)
        .expect("cull runs");

    assert_eq!(culled, 1, "exactly the expired dir is culled");
    assert!(!stale.exists(), "a dir 40 days old is past a 30-day retention period");
    assert!(
        fresh.exists(),
        "and a fresh one stays, because the cap was never the question"
    );
}

#[test]
fn a_longer_retention_period_keeps_what_a_shorter_one_would_cull() {
    // The setting is the rule, not a decoration on top of a fixed one.
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let stale = state
        .run_dir
        .join("sessions")
        .join("vm-old-failed-20260101-000000-aaaa");
    std::fs::create_dir_all(&stale).unwrap();
    let forty_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(40 * 86_400);
    filetime::set_file_mtime(&stale, filetime::FileTime::from_system_time(forty_days_ago)).unwrap();

    assert_eq!(state.cull_failed_sessions_older_than(365).expect("cull runs"), 0);
    assert!(stale.exists(), "a year of retention keeps a 40-day-old dir");

    assert_eq!(state.cull_failed_sessions_older_than(7).expect("cull runs"), 1);
    assert!(!stale.exists(), "a week of it does not");
}

#[test]
fn retention_days_falls_back_to_the_declared_default() {
    use capsem_core::net::policy_config::SettingValue;

    assert_eq!(
        retention_days_from_resolved(&[]),
        DEFAULT_RETENTION_DAYS,
        "an unset setting is the documented default, not zero"
    );
    assert_eq!(
        retention_days_from_resolved(&[resolved_retention_days(SettingValue::Number(7))]),
        7
    );
    assert_eq!(
        retention_days_from_resolved(&[resolved_retention_days(SettingValue::Number(0))]),
        DEFAULT_RETENTION_DAYS,
        "zero would mean deleting evidence as it is written; the setting's own floor is 1"
    );
    assert_eq!(
        retention_days_from_resolved(&[resolved_retention_days(SettingValue::Text("thirty".into()))]),
        DEFAULT_RETENTION_DAYS,
        "a value of the wrong type is not a retention period"
    );
}

fn resolved_retention_days(
    value: capsem_core::net::policy_config::SettingValue,
) -> capsem_core::net::policy_config::ResolvedSetting {
    capsem_core::net::policy_config::ResolvedSetting {
        id: "vm.resources.retention_days".to_string(),
        category: "VM".to_string(),
        name: "Session retention".to_string(),
        description: String::new(),
        setting_type: capsem_core::net::policy_config::SettingType::Number,
        default_value: capsem_core::net::policy_config::SettingValue::Number(30),
        effective_value: value,
        source: capsem_core::net::policy_config::PolicySource::User,
        modified: None,
        corp_locked: false,
        enabled_by: None,
        enabled: true,
        metadata: capsem_core::net::policy_config::SettingMetadata::default(),
        collapsed: false,
        history: Vec::new(),
    }
}

/// The service keeps a session's DB handle registered until the reaper
/// unregisters it, and that is not always before the VM's own process has
/// finished shutting down: on the normal stop path the handle goes first, but
/// a persistent process that exits on its own is reaped afterwards. Retention
/// runs during that shutdown and replaces the archive file, so for that window
/// the service is holding a reader on an inode that is no longer the archive.
///
/// A route reading a body in that window must get the body, not an integrity
/// failure about data that is perfectly intact.
#[tokio::test]
async fn a_registered_session_handle_reads_bodies_after_the_process_trims_them() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let session_dir = state.run_dir.join("sessions").join("persistent-vm");
    std::fs::create_dir_all(&session_dir).unwrap();

    // The ledger as capsem-process owns it.
    let owner = capsem_logger::DbHandle::open(&session_dir.join("session.db")).expect("open the owning handle");
    let write_body = |event_id: &'static str, payload: &'static str| {
        let owner = &owner;
        async move {
            owner
                .write(capsem_logger::WriteOp::SecurityRuleEvent(
                    capsem_logger::SecurityRuleEvent::new(
                        1_789_000_223_456,
                        event_id,
                        "model.call",
                        "profiles.rules.example",
                        r#"{"name":"example"}"#,
                        payload,
                    ),
                ))
                .await
                .expect("write a rule event");
            owner.flush().await.expect("flush");
        }
    };
    write_body("0000000000ab", r#"{"old":1}"#).await;
    write_body("0000000000cd", r#"{"new":2}"#).await;

    // The service registers its external reader and serves a body from it,
    // which is what leaves it holding the archive open.
    let handle = state
        .register_session_db_handle("persistent-vm", &session_dir)
        .expect("register the session handle");
    assert_eq!(
        handle
            .read_body("0000000000cd", capsem_logger::BodyDirection::Payload)
            .await
            .expect("read a body")
            .expect("the body is archived")
            .bytes,
        br#"{"new":2}"#
    );

    // The VM's process trims on its way out. The handle above is still
    // registered: this is the window the reaper leaves open.
    let cutoff = {
        let raw = handle
            .query("SELECT sealed_at FROM body_blocks ORDER BY block_offset", &[])
            .await
            .expect("read the block seal times");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("rows");
        value["rows"][1][0].as_str().expect("the newer seal time").to_string()
    };
    owner.retain_bodies_since(&cutoff).await.expect("trim archived bodies");

    assert_eq!(
        handle
            .read_body("0000000000cd", capsem_logger::BodyDirection::Payload)
            .await
            .expect("a still-registered handle must follow the archive, not fail on it")
            .expect("the surviving body is still archived")
            .bytes,
        br#"{"new":2}"#,
        "the same read returns the same bytes after the file was replaced under it"
    );
    assert!(
        handle
            .read_body("0000000000ab", capsem_logger::BodyDirection::Payload)
            .await
            .expect("read the dropped body")
            .is_none(),
        "and a dropped body is absent rather than stale bytes from the old inode"
    );

    state.unregister_session_db_handle("persistent-vm");
}
