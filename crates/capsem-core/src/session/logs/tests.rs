use super::*;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

#[test]
fn migrating_keeps_every_generation_and_reopening_appends() {
    let session = tempfile::tempdir().unwrap();
    let root = session.path();
    for (name, bytes) in [
        ("serial.log", b"current\n".as_slice()),
        ("serial.1.log", b"older\n".as_slice()),
    ] {
        std::fs::write(root.join(name), bytes).unwrap();
    }
    let old_pty = crate::pty_log::PtyLog::open(&root.join("pty.log.1")).unwrap();
    old_pty.record_output(b"old output\n");
    drop(old_pty);
    let pty = crate::pty_log::PtyLog::open(&root.join("pty.log")).unwrap();
    pty.record_input(b"secret input\n");
    pty.record_output(b"current output\n");
    drop(pty);
    let originals = LOG_NAMES.map(|name| std::fs::read(root.join(name)).unwrap());
    let logs = prepare_session_logs(root).unwrap();
    for (index, name) in LOG_NAMES.iter().enumerate() {
        assert!(!root.join(name).exists());
        let migrated = root.join(OWNER_STATE_DIR).join(LOG_DIR).join(name);
        assert_eq!(std::fs::read(&migrated).unwrap(), originals[index]);
        assert_eq!(std::fs::metadata(migrated).unwrap().permissions().mode() & 0o777, 0o600);
    }
    logs.open_serial().unwrap().write_all(b"appended\n").unwrap();
    logs.open_pty().unwrap().record_output(b"more output\n");
    prepare_session_logs(root).unwrap();
    assert_eq!(
        read_session_serial_tail(root, 1024).unwrap().unwrap(),
        b"older\ncurrent\nappended\n"
    );
    assert_eq!(
        read_session_transcript(root).unwrap().unwrap(),
        b"old output\ncurrent output\nmore output\n"
    );
}

#[test]
fn unmigrated_sessions_are_read_without_creating_a_new_layout() {
    let session = tempfile::tempdir().unwrap();
    std::fs::write(session.path().join("serial.log"), b"legacy console\n").unwrap();
    crate::pty_log::PtyLog::open(&session.path().join("pty.log"))
        .unwrap()
        .record_output(b"legacy output\n");
    assert_eq!(
        read_session_serial_tail(session.path(), 1024).unwrap().unwrap(),
        b"legacy console\n"
    );
    assert_eq!(
        read_session_transcript(session.path()).unwrap().unwrap(),
        b"legacy output\n"
    );
    assert!(!session.path().join(OWNER_STATE_DIR).exists());
}

#[test]
fn migration_conflicts_leave_all_original_files_unchanged() {
    let session = tempfile::tempdir().unwrap();
    let destination = session.path().join(OWNER_STATE_DIR).join(LOG_DIR);
    prepare_session_logs(session.path()).unwrap();
    std::fs::write(session.path().join("serial.log"), b"legacy").unwrap();
    std::fs::write(session.path().join("pty.log.1"), b"legacy archive").unwrap();
    std::fs::write(destination.join("serial.log"), b"current").unwrap();
    assert_eq!(
        prepare_session_logs(session.path()).err().unwrap().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(session.path().join("serial.log")).unwrap(), b"legacy");
    assert_eq!(std::fs::read(destination.join("serial.log")).unwrap(), b"current");
    assert_eq!(
        std::fs::read(session.path().join("pty.log.1")).unwrap(),
        b"legacy archive"
    );
    assert!(!destination.join("pty.log.1").exists());
}

#[test]
fn a_redirected_log_directory_never_falls_back_to_legacy_files() {
    for component in [OWNER_STATE_DIR, LOG_DIR] {
        let session = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let parent = if component == OWNER_STATE_DIR {
            session.path().to_path_buf()
        } else {
            let owner = session.path().join(OWNER_STATE_DIR);
            std::fs::create_dir(&owner).unwrap();
            owner
        };
        std::os::unix::fs::symlink(outside.path(), parent.join(component)).unwrap();
        std::fs::write(session.path().join("serial.log"), b"legacy").unwrap();
        assert!(prepare_session_logs(session.path()).is_err());
        assert!(read_session_serial_tail(session.path(), 1024).is_err());
        assert!(read_session_transcript(session.path()).is_err());
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    }
}

#[test]
fn linked_generations_are_refused_without_changing_the_target() {
    for name in LOG_NAMES {
        for symbolic in [true, false] {
            let session = tempfile::tempdir().unwrap();
            let outside = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(outside.path(), b"host-only fixture").unwrap();
            let target = session.path().join(name);
            if symbolic {
                std::os::unix::fs::symlink(outside.path(), &target).unwrap();
            } else {
                std::fs::hard_link(outside.path(), &target).unwrap();
            }
            assert!(prepare_session_logs(session.path()).is_err());
            assert_eq!(std::fs::read(outside.path()).unwrap(), b"host-only fixture");
        }
    }
}

#[test]
fn readers_refuse_redirected_generations_in_the_owner_layout() {
    for name in LOG_NAMES {
        for symbolic in [true, false] {
            let session = tempfile::tempdir().unwrap();
            let outside = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(outside.path(), b"host-only fixture").unwrap();
            prepare_session_logs(session.path()).unwrap();
            let target = session.path().join(OWNER_STATE_DIR).join(LOG_DIR).join(name);
            if target.exists() {
                std::fs::remove_file(&target).unwrap();
            }
            if symbolic {
                std::os::unix::fs::symlink(outside.path(), &target).unwrap();
            } else {
                std::fs::hard_link(outside.path(), &target).unwrap();
            }
            let result = if name.starts_with("serial") {
                read_session_serial_tail(session.path(), 1024)
            } else {
                read_session_transcript(session.path())
            };
            assert!(result.is_err());
            assert!(prepare_session_logs(session.path()).is_err());
            assert_eq!(std::fs::read(outside.path()).unwrap(), b"host-only fixture");
        }
    }
}

#[test]
fn missing_logs_are_empty_and_serial_tails_stay_bounded() {
    let session = tempfile::tempdir().unwrap();
    assert!(read_session_serial_tail(session.path(), 1024).unwrap().is_none());
    assert!(read_session_transcript(session.path()).unwrap().is_none());
    let logs = prepare_session_logs(session.path()).unwrap();
    logs.open_serial().unwrap().write_all(b"0123456789").unwrap();
    assert_eq!(read_session_serial_tail(session.path(), 4).unwrap().unwrap(), b"6789");
    assert!(read_session_transcript(session.path()).unwrap().unwrap().is_empty());
}
