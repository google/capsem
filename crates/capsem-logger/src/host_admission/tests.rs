use super::*;
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;

fn account() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().canonicalize().unwrap().join("account ?&%# name");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
    (root, home)
}

fn current(home: &Path, wal: bool) -> Connection {
    let dir = home.join(".capsem/sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let conn = Connection::open(dir.join("host.db")).unwrap();
    if wal {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
    }
    crate::schema::create_tables(&conn).unwrap();
    conn.execute(
        "INSERT INTO host_events(timestamp_unix_ms,kind,actor,detail) VALUES(1,'service_started','fixture',x'80')",
        [],
    )
    .unwrap();
    conn
}

fn files(home: &Path) -> BTreeMap<String, Vec<u8>> {
    let dir = home.join(".capsem/sessions");
    if !dir.exists() {
        return BTreeMap::new();
    }
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn actual_active_and_stopped_wal_admit_without_ledger_payload_changes() {
    let (_root, home) = account();
    let writer = current(&home, true);
    assert!(home.join(".capsem/sessions/host.db-wal").is_file());
    assert!(home.join(".capsem/sessions/host.db-shm").is_file());
    let before = files(&home);
    admit_separately(&home, true);
    assert_ledger_payload_unchanged(&home, &before);
    writer
        .execute(
            "INSERT INTO host_events(timestamp_unix_ms,kind,actor,detail) VALUES(2,'service_started','fixture',x'80')",
            [],
        )
        .unwrap();
    let before = files(&home);
    admit_separately(&home, true);
    assert_ledger_payload_unchanged(&home, &before);
    drop(writer);
    let before = files(&home);
    assert!(!home.join(".capsem/sessions/host.db-shm").exists());
    admit_separately(&home, true);
    assert_ledger_payload_unchanged(&home, &before);
    // A stopped rollback-journal schema needs no SHM recovery and admits.
    let (_root, stopped_home) = account();
    drop(current(&stopped_home, false));
    let before = files(&stopped_home);
    admit_separately(&stopped_home, true);
    assert_ledger_payload_unchanged(&stopped_home, &before);
}

#[test]
fn absent_state_is_distinct_from_partial_or_corrupt_existing_state() {
    let (_root, home) = account();
    assert_eq!(
        admit_preinstall_host_schema(&home, Duration::from_secs(1)).unwrap(),
        HostSchemaAdmission::Absent
    );
    assert!(!home.join(".capsem").exists());
    std::fs::create_dir_all(home.join(".capsem/sessions")).unwrap();
    let db = home.join(".capsem/sessions/host.db");
    for data in [b"".as_slice(), b"not SQLite".as_slice()] {
        std::fs::write(&db, data).unwrap();
        assert!(admit_preinstall_host_schema(&home, Duration::from_secs(1)).is_err());
        assert_eq!(std::fs::read(&db).unwrap(), data);
    }
    std::fs::remove_file(&db).unwrap();
    for name in ["host.db-wal", "host.db-shm", "host.db-journal", "main.db"] {
        let partial = db.with_file_name(name);
        std::fs::write(&partial, b"original partial").unwrap();
        assert!(admit_preinstall_host_schema(&home, Duration::from_secs(1)).is_err());
        assert_eq!(std::fs::read(&partial).unwrap(), b"original partial");
        std::fs::remove_file(partial).unwrap();
    }
}

#[test]
fn old_hash_schema_refuses_without_modifying_original_bytes() {
    let (_root, home) = account();
    let writer = current(&home, false);
    writer
        .execute_batch("ALTER TABLE host_events ADD COLUMN prev_hash BLOB")
        .unwrap();
    drop(writer);
    let before = files(&home);
    let error = admit_preinstall_host_schema(&home, Duration::from_secs(1))
        .unwrap_err()
        .to_string();
    assert!(error.contains("obsolete host hash-chain"), "{error}");
    assert_ledger_payload_unchanged(&home, &before);
}

#[test]
fn symlinks_and_external_write_authority_never_admit() {
    for segment in [
        "account",
        ".capsem",
        "sessions",
        "host.db",
        "host.db-wal",
        "host.db-shm",
    ] {
        let (root, home) = account();
        drop(current(&home, false));
        let target = match segment {
            "account" => home.clone(),
            ".capsem" => home.join(".capsem"),
            "sessions" => home.join(".capsem/sessions"),
            name => home.join(".capsem/sessions").join(name),
        };
        let saved = root.path().join("saved");
        if target.exists() {
            std::fs::rename(&target, &saved).unwrap();
        } else {
            std::fs::write(&saved, b"not a real sidecar").unwrap();
        }
        symlink(&saved, &target).unwrap();
        assert!(
            admit_preinstall_host_schema(&home, Duration::from_secs(1)).is_err(),
            "{segment}"
        );
    }
    for relative in ["", ".capsem", ".capsem/sessions", ".capsem/sessions/host.db"] {
        let (_root, home) = account();
        drop(current(&home, false));
        let unsafe_path = home.join(relative);
        std::fs::set_permissions(&unsafe_path, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(admit_preinstall_host_schema(&home, Duration::from_secs(1)).is_err());
        assert_eq!(
            std::fs::metadata(unsafe_path).unwrap().permissions().mode() & 0o777,
            0o777
        );
    }
}

#[test]
fn nonpositive_or_excessive_budget_refuses_without_creating_state() {
    let (_root, home) = account();
    for timeout in [Duration::ZERO, Duration::from_secs(6)] {
        assert!(admit_preinstall_host_schema(&home, timeout).is_err());
        assert!(!home.join(".capsem").exists());
    }
}

fn assert_ledger_payload_unchanged(home: &Path, before: &BTreeMap<String, Vec<u8>>) {
    let after = files(home);
    for (name, bytes) in before {
        if name == "host.db-shm" {
            continue;
        } // SQLite coordination, not ledger payload.
        let actual = &after[name];
        assert_eq!(actual.len(), bytes.len(), "{name} length changed");
        let offsets: Vec<_> = actual
            .iter()
            .zip(bytes)
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i))
            .take(32)
            .collect();
        assert!(offsets.is_empty(), "{name} payload changed at offsets {offsets:?}");
    }
    for (name, bytes) in &after {
        if !before.contains_key(name) {
            assert!(
                name == "host.db-shm" || (name == "host.db-wal" && bytes.is_empty()),
                "unexpected new ledger state {name}"
            );
        }
    }
}

#[test]
fn admission_candidate_child() {
    let Some(home) = std::env::var_os("CAPSEM_SCHEMA_ADMISSION_TEST_HOME") else {
        return;
    };
    let result = admit_preinstall_host_schema(Path::new(&home), Duration::from_secs(1));
    if std::env::var_os("CAPSEM_SCHEMA_ADMISSION_TEST_REFUSE").is_some() {
        assert!(result.is_err(), "partial/recovery state unexpectedly admitted");
    } else {
        assert_eq!(result.unwrap(), HostSchemaAdmission::Current);
    }
}

fn admit_separately(home: &Path, expected: bool) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    if !expected {
        command.env("CAPSEM_SCHEMA_ADMISSION_TEST_REFUSE", "1");
    }
    let status = command
        .args([
            "--exact",
            "host_admission::tests::admission_candidate_child",
            "--nocapture",
        ])
        .env("CAPSEM_SCHEMA_ADMISSION_TEST_HOME", home)
        .status()
        .unwrap();
    assert!(status.success(), "isolated admission candidate refused current schema");
}

#[test]
fn partial_archive_and_retired_state_never_become_fresh() {
    let (_root, home) = account();
    std::fs::create_dir_all(home.join(".capsem/sessions/host.bodies")).unwrap();
    assert!(admit_preinstall_host_schema(&home, Duration::from_secs(1)).is_err());
}

#[test]
fn exclusive_writer_lock_obeys_remaining_budget_without_mutation() {
    let (_root, home) = account();
    let writer = current(&home, false);
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let before = files(&home);
    let start = std::time::Instant::now();
    assert!(admit_preinstall_host_schema(&home, Duration::from_millis(30)).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_ledger_payload_unchanged(&home, &before);
    writer.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn truncated_uncheckpointed_wal_cannot_admit_missing_schema() {
    let (_root, home) = account();
    let writer = current(&home, true);
    let wal = home.join(".capsem/sessions/host.db-wal");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&wal)
        .unwrap()
        .set_len(32)
        .unwrap();
    let before = files(&home);
    admit_separately(&home, false);
    assert_ledger_payload_unchanged(&home, &before);
    drop(writer);
}

#[test]
fn separate_candidate_reads_snapshots_during_actual_row_writes() {
    let (_root, home) = account();
    let writer = current(&home, true);
    let writer = std::thread::spawn(move || {
        for index in 0..100 {
            writer.execute("INSERT INTO host_events(timestamp_unix_ms,kind,actor,detail) VALUES(?,'service_started','fixture',x'80')", [index]).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        writer
    });
    for _ in 0..5 {
        admit_separately(&home, true);
    }
    let writer = writer.join().unwrap();
    assert_eq!(
        writer
            .query_row("SELECT count(*) FROM host_events", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        101
    );
    // Producer is now idle: byte equality is meaningful only in this interval.
    let before = files(&home);
    admit_separately(&home, true);
    assert_ledger_payload_unchanged(&home, &before);
}
