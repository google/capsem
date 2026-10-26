use super::*;
use capsem_archive::{format, GenerationId};
use capsem_foundation::unix::lock::{self, LockMode};

fn ledger() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let (root, home) = account();
    sessions(&home);
    let db = home.join(".capsem/sessions/host.db");
    let writer = crate::DbWriter::open(&db, 8).unwrap();
    writer.shutdown_blocking();
    let conn = Connection::open(&db).unwrap();
    let state = crate::schema::archive_state(&conn).unwrap();
    let archive = crate::writer::archive_path_for_db(&db).join(state.header.generation_id.file_name());
    (root, home, archive)
}

fn original(home: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                collect(&entry.path(), out);
            } else if entry.file_name() != "host.db-shm" {
                out.insert(entry.path(), std::fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    collect(&home.join(".capsem/sessions"), &mut out);
    out
}

fn unchanged(before: &BTreeMap<PathBuf, Vec<u8>>) {
    for (path, bytes) in before {
        assert!(std::fs::read(path).unwrap() == *bytes, "original payload changed");
    }
}

#[test]
fn current_physical_archive_admits_without_recovery_or_rewriting() {
    let (_root, home, archive) = ledger();
    // Uncommitted tail is not scanned, decoded, repaired or truncated.
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&archive)
        .unwrap()
        .write_all(b"uncommitted tail")
        .unwrap();
    let before = original(&home);
    assert_eq!(
        admit_preinstall_host_state(&home, Duration::from_secs(1)).unwrap(),
        HostSchemaAdmission::Current
    );
    unchanged(&before);
}

#[test]
fn missing_corrupt_wrong_generation_and_header_version_refuse_unchanged() {
    for case in [
        "missing",
        "magic",
        "generation",
        "archive",
        "version",
        "truncated",
        "extent",
    ] {
        let (_root, home, archive) = ledger();
        let db = home.join(".capsem/sessions/host.db");
        match case {
            "missing" => std::fs::remove_file(&archive).unwrap(),
            "magic" => {
                let mut bytes = std::fs::read(&archive).unwrap();
                bytes[0] ^= 1;
                std::fs::write(&archive, bytes).unwrap();
            }
            "version" => {
                let mut bytes = std::fs::read(&archive).unwrap();
                bytes[8] = 255;
                std::fs::write(&archive, bytes).unwrap();
            }
            "archive" => {
                let conn = Connection::open(&db).unwrap();
                let state = crate::schema::archive_state(&conn).unwrap();
                std::fs::write(
                    &archive,
                    format::encode_file_header(capsem_archive::ArchiveId::generate(), state.header.generation_id),
                )
                .unwrap();
            }
            "generation" => {
                let conn = Connection::open(&db).unwrap();
                let state = crate::schema::archive_state(&conn).unwrap();
                std::fs::write(
                    &archive,
                    format::encode_file_header(state.header.archive_id, GenerationId::generate()),
                )
                .unwrap();
            }
            "truncated" => {
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&archive)
                    .unwrap()
                    .set_len(12)
                    .unwrap();
            }
            "extent" => {
                let conn = Connection::open(&db).unwrap();
                conn.execute("UPDATE archive_state SET committed_end=committed_end+1", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before = original(&home);
        assert!(
            admit_preinstall_host_state(&home, Duration::from_secs(1)).is_err(),
            "{case}"
        );
        unchanged(&before);
    }
}

#[test]
fn missing_or_busy_generation_lock_never_becomes_a_fresh_install() {
    let (_root, home, _archive) = ledger();
    let path = crate::writer::archive_lock_path_for_db(&home.join(".capsem/sessions/host.db"));
    let held = lock::acquire(&path, LockMode::Exclusive).unwrap();
    let before = original(&home);
    let start = std::time::Instant::now();
    assert!(admit_preinstall_host_state(&home, Duration::from_millis(30)).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    unchanged(&before);
    drop(held);
    std::fs::remove_file(path).unwrap();
    assert!(admit_preinstall_host_state(&home, Duration::from_secs(1)).is_err());
}

#[test]
fn active_writer_and_selected_generation_symlink_preserve_payloads() {
    let (_root, home, archive) = ledger();
    let db = home.join(".capsem/sessions/host.db");
    let writer = crate::DbWriter::open(&db, 8).unwrap();
    let before = original(&home);
    assert_eq!(
        admit_preinstall_host_state(&home, Duration::from_secs(1)).unwrap(),
        HostSchemaAdmission::Current
    );
    unchanged(&before);
    writer.shutdown_blocking();
    let saved = archive.with_extension("saved");
    std::fs::rename(&archive, &saved).unwrap();
    symlink(&saved, &archive).unwrap();
    assert!(admit_preinstall_host_state(&home, Duration::from_secs(1)).is_err());
    assert_eq!(std::fs::read(&saved).unwrap(), std::fs::read(&archive).unwrap());
}
