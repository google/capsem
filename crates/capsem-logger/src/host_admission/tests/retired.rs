use super::*;
use std::os::unix::fs::MetadataExt;

const FAMILY: [&str; 4] = ["main.db", "main.db-wal", "main.db-shm", "main.db-journal"];

fn dormant(home: &Path) {
    let directory = sessions(home);
    for name in FAMILY {
        // Deliberately not SQLite: admission must never interpret retired data.
        fs::write_new_regular_file_no_follow(
            &directory.join(name), format!("original inert {name}").as_bytes(), 0o600,
        ).unwrap();
    }
}

type Original = (Vec<u8>, u64, u64, u32, i64, i64);
fn originals(home: &Path) -> BTreeMap<String, Original> {
    FAMILY
        .into_iter()
        .filter_map(|name| {
            let path = home.join(".capsem/sessions").join(name);
            let metadata = std::fs::metadata(&path).ok()?;
            Some((
                name.to_owned(),
                (
                    std::fs::read(path).unwrap(),
                    metadata.dev(),
                    metadata.ino(),
                    metadata.mode(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                ),
            ))
        })
        .collect()
}

#[test]
fn regular_retired_main_only_is_absent_current_host_without_any_old_sqlite_open() {
    let (_root, home) = account();
    dormant(&home);
    for name in &FAMILY[1..] {
        std::fs::remove_file(home.join(".capsem/sessions").join(name)).unwrap();
    }
    let before = originals(&home);
    assert_eq!(
        admit_preinstall_host_state(&home, Duration::from_secs(1)).unwrap(),
        HostSchemaAdmission::Absent
    );
    assert_eq!(originals(&home), before);
    assert!(!home.join(".capsem/sessions/host.db").exists());
}

#[test]
fn inert_main_family_and_current_physical_host_coexist_unchanged() {
    for host_present in [false, true] {
        let (_root, home) = account();
        dormant(&home);
        if host_present {
            crate::DbWriter::open(&home.join(".capsem/sessions/host.db"), 8)
                .unwrap()
                .shutdown_blocking();
        }
        let before = originals(&home);
        let result = admit_preinstall_host_state(&home, Duration::from_secs(1)).unwrap();
        assert_eq!(
            result,
            if host_present {
                HostSchemaAdmission::Current
            } else {
                HostSchemaAdmission::Absent
            }
        );
        assert_eq!(originals(&home), before);
    }
}

#[test]
fn live_retired_writer_transaction_is_not_opened_or_interfered_with() {
    let (_root, home) = account();
    let directory = sessions(&home);
    let old = Connection::open(directory.join("main.db")).unwrap();
    old.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE original(id INTEGER); INSERT INTO original VALUES(1); BEGIN IMMEDIATE; INSERT INTO original VALUES(2)").unwrap();
    let before = originals(&home);
    assert_eq!(
        admit_preinstall_host_state(&home, Duration::from_secs(1)).unwrap(),
        HostSchemaAdmission::Absent
    );
    assert_eq!(originals(&home), before);
    old.execute_batch("COMMIT").unwrap();
    assert_eq!(
        old.query_row("SELECT COUNT(*) FROM original", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn unsafe_retired_family_never_gains_path_authority() {
    for name in FAMILY {
        for defect in ["symlink", "directory", "hardlink", "writable"] {
            let (root, home) = account();
            dormant(&home);
            let path = home.join(".capsem/sessions").join(name);
            if defect == "writable" {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
            } else {
                std::fs::remove_file(&path).unwrap();
                let outside = root.path().join("outside");
                std::fs::write(&outside, b"foreign original").unwrap();
                match defect {
                    "symlink" => symlink(&outside, &path).unwrap(),
                    "hardlink" => ContainedDir::open_root(root.path())
                        .unwrap()
                        .hard_link(
                            OsStr::new("outside"),
                            &ContainedDir::open_root(path.parent().unwrap()).unwrap(),
                            OsStr::new(name),
                        )
                        .unwrap(),
                    "directory" => std::fs::create_dir(&path).unwrap(),
                    _ => unreachable!(),
                }
            }
            assert!(
                admit_preinstall_host_schema(&home, Duration::from_secs(1)).is_err(),
                "{name}: {defect}"
            );
        }
    }
}

#[test]
fn retired_files_do_not_mask_incompatible_current_host_or_orphan_host_sidecar() {
    for defect in ["hash", "corrupt", "orphan"] {
        let (_root, home) = account();
        dormant(&home);
        if defect == "hash" {
            let writer = current(&home, false);
            writer
                .execute_batch("ALTER TABLE host_events ADD COLUMN prev_hash BLOB")
                .unwrap();
        } else {
            std::fs::write(
                home.join(".capsem/sessions")
                    .join(if defect == "corrupt" { "host.db" } else { "host.db-wal" }),
                b"unchanged incompatible host",
            )
            .unwrap();
        }
        let before = originals(&home);
        assert!(
            admit_preinstall_host_schema(&home, Duration::from_secs(1)).is_err(),
            "{defect}"
        );
        assert_eq!(originals(&home), before);
    }
}

#[test]
fn orphan_retired_sidecars_cannot_be_mistaken_for_absent_state() {
    for name in &FAMILY[1..] {
        let (_root, home) = account();
        let directory = home.join(".capsem/sessions");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(name), b"retained orphan bytes").unwrap();
        let before = originals(&home);
        assert!(admit_preinstall_host_state(&home, Duration::from_secs(1)).is_err());
        assert_eq!(originals(&home), before);
    }
}
