use super::*;
use crate::db::BodyDirection;
use crate::events::{ExecEvent, ExecEventComplete};
use crate::host_admission::{admit_preinstall_host_state, HostSchemaAdmission};
use crate::{DbHandle, WriteOp};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::time::{Duration, SystemTime};

const EXEC_EVENT: &str = "0123456789ab";
const OUTPUT: &[u8] = b"output archived by the previous release\xff";

/// Apply the inverse of the three declaration changes since the v0.7.2 tag,
/// while retaining the data and real archive already produced by the fixture.
fn use_released_schema(conn: &Connection) {
    let mut prior = CREATE_SCHEMA
        .strip_suffix(include_str!("../environment.sql"))
        .unwrap()
        .replace(", 'environment.revision', 'environment.request'", "")
        .replace(", 'environment_revisions', 'environment_requests'", "")
        .replace(", 'access_denied'", "");
    let start = prior.find(",\n            'environment_revision_prepared'").unwrap();
    let end = start + prior[start..].find("\n        )),").unwrap();
    prior.replace_range(start..end, "");

    conn.pragma_update(None, "foreign_keys", false).unwrap();
    for (table, _) in super::super::environment::tables() {
        conn.execute_batch(&format!("DROP TABLE main.{table}")).unwrap();
    }
    conn.execute_batch(
        "CREATE TEMP TABLE previous_hosts AS SELECT * FROM host_events;
         CREATE TEMP TABLE previous_bodies AS SELECT * FROM event_body_blobs;
         DROP TABLE main.host_events;
         DROP TABLE main.event_body_blobs;",
    )
    .unwrap();
    conn.execute_batch(&prior).unwrap();
    conn.execute_batch(
        "INSERT INTO main.host_events SELECT * FROM temp.previous_hosts;
         INSERT INTO main.event_body_blobs SELECT * FROM temp.previous_bodies;
         UPDATE main.sqlite_sequence SET seq=20 WHERE name='host_events';
         UPDATE main.sqlite_sequence SET seq=30 WHERE name='event_body_blobs';",
    )
    .unwrap();
}

#[tokio::test]
async fn published_072_ledger_admits_read_only_and_upgrades_with_history() {
    let home = tempfile::tempdir().unwrap();
    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let sessions = home.path().join(".capsem/sessions");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(&sessions)
        .unwrap();
    let path = sessions.join("host.db");
    let db = DbHandle::open(&path).unwrap();
    db.write(WriteOp::ExecEvent(ExecEvent {
        event_id: Some(EXEC_EVENT.into()),
        timestamp: SystemTime::now(),
        exec_id: 72,
        command: "fixture".into(),
        source: "api".into(),
        target: capsem_proto::ipc::ExecTarget::Vm,
        trace_id: None,
        process_name: None,
        credential_ref: None,
    }))
    .await
    .unwrap();
    db.write(WriteOp::ExecEventComplete(ExecEventComplete {
        exec_id: 72,
        exit_code: 0,
        duration_ms: 1,
        stdout: OUTPUT.to_vec(),
        stderr: Vec::new(),
        stdout_bytes: OUTPUT.len() as u64,
        stderr_bytes: 0,
        pid: None,
    }))
    .await
    .unwrap();
    db.flush().await.unwrap();
    drop(db);

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO host_events (id,timestamp_unix_ms,kind,actor,detail)
         VALUES (7,1,'service_started','fixture',x'80')",
        [],
    )
    .unwrap();
    use_released_schema(&conn);
    let state = archive_state(&conn).unwrap();
    let archive = crate::writer::archive_path_for_db(&path).join(state.header.generation_id.file_name());
    let contents = std::fs::read(&archive).unwrap();
    let counters = counter_bytes(&conn);
    assert!(validate_existing_host_schema(&conn).is_err());
    conn.close().unwrap();
    for entry in std::fs::read_dir(&sessions).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        admit_preinstall_host_state(home.path(), Duration::from_secs(1)).expect("published 0.7.2 ledger must admit"),
        HostSchemaAdmission::ReleasedV072,
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);

    let db = DbHandle::open(&path).unwrap();
    let payload = db
        .read_body(EXEC_EVENT, "exec_events", BodyDirection::Stdout)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(payload.bytes, OUTPUT);
    drop(db);
    let conn = Connection::open(&path).unwrap();
    validate_existing_host_schema(&conn).unwrap();
    assert_eq!(std::fs::read(&archive).unwrap(), contents);
    assert_eq!(archive_state(&conn).unwrap().header, state.header);
    assert_eq!(archive_state(&conn).unwrap().committed_end, state.committed_end);
    assert_eq!(counter_bytes(&conn), counters);
    assert_eq!(
        conn.query_row("SELECT detail FROM host_events WHERE id=7", [], |r| r
            .get::<_, Vec<u8>>(0))
            .unwrap(),
        [0x80],
    );
    for (table, seq) in [("host_events", 20), ("event_body_blobs", 30)] {
        assert_eq!(
            conn.query_row("SELECT seq FROM sqlite_sequence WHERE name=?1", [table], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            seq,
        );
    }
}

fn counter_bytes(conn: &Connection) -> Vec<u8> {
    conn.query_row("SELECT counters FROM ledger_counters WHERE singleton=1", [], |row| {
        row.get(0)
    })
    .unwrap()
}

#[test]
fn released_ledger_with_an_unknown_schema_object_is_not_admitted() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    use_released_schema(&conn);
    assert!(super::super::v072::is_supported_predecessor(&conn).unwrap());
    conn.execute_batch("CREATE VIEW custom_host_references AS SELECT id FROM host_events")
        .unwrap();
    assert!(!super::super::v072::is_supported_predecessor(&conn).unwrap());
    assert!(super::super::v072::upgrade(&conn).is_err());
    assert!(conn.is_autocommit());
}

#[test]
fn failed_archive_index_check_rolls_back_the_whole_transition() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    use_released_schema(&conn);
    conn.execute_batch(
        "INSERT INTO host_events (id,timestamp_unix_ms,kind,actor,detail)
         VALUES (7,1,'service_started','fixture',x'80');
         INSERT INTO event_body_blobs (
            event_id,event_type,source_table,direction,original_bytes,stored_bytes,
            truncated,body_hash,block_offset,body_offset,body_len,created_at
         ) VALUES (
            '0123456789ab','process.exec','exec_events','stdout',1,1,0,
            'xxh3-128:00000000000000000000000000000000',1234,0,1,'fixture'
         );",
    )
    .unwrap();
    let error = super::super::v072::upgrade(&conn).unwrap_err().to_string();
    assert!(error.contains("invalid archive index reference"), "{error}");
    assert!(super::super::v072::is_supported_predecessor(&conn).unwrap());
    assert!(conn.is_autocommit());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM host_events WHERE id=7", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1,
    );
}
