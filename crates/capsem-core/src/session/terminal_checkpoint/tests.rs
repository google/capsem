use super::*;
use crate::terminal::SessionTerminal;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

fn checkpoint_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join("owner/terminal/checkpoint.msgpack")
}

#[test]
fn durable_private_checkpoint_reopens_with_model_and_effect_identity() {
    let root = tempfile::tempdir().unwrap();
    let store = TerminalCheckpointStore::prepare(root.path()).unwrap();
    assert!(
        store.load(8).is_err(),
        "missing continuation must not become a blank screen"
    );
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"primary\x1b[?1049halt\x07\x1b]0;Stored\x07\x1b]777;notify;Title;Body\x07\x1b[38;2;");
    store.save(&terminal).unwrap();
    let file = std::fs::metadata(checkpoint_path(root.path())).unwrap();
    assert_eq!(file.mode() & 0o777, 0o600);
    assert_eq!(file.nlink(), 1);
    assert_eq!(
        std::fs::metadata(root.path().join("owner/terminal")).unwrap().mode() & 0o777,
        0o700
    );
    drop(store);
    let reopened = TerminalCheckpointStore::prepare(root.path()).unwrap();
    let restored = reopened.load(8).unwrap();
    assert_eq!(terminal.subscribe().1.render, restored.subscribe().1.render);
    assert_eq!(terminal.subscribe().1.state, restored.subscribe().1.state);
    assert_eq!(
        *terminal.subscribe_summary().borrow(),
        *restored.subscribe_summary().borrow()
    );
    assert_eq!(
        terminal.publish(b"10;20;30mRGB\x1b[?1049l"),
        restored.publish(b"10;20;30mRGB\x1b[?1049l")
    );
    assert_eq!(terminal.subscribe().1.render, restored.subscribe().1.render);
    reopened.save(&restored).unwrap();
    assert_eq!(
        reopened.load(8).unwrap().subscribe().1.state,
        restored.subscribe().1.state
    );
    assert_eq!(
        std::fs::read_dir(root.path().join("owner/terminal")).unwrap().count(),
        1
    );
    reopened.clear().unwrap();
    reopened.clear().unwrap();
    assert!(!checkpoint_path(root.path()).exists());
}

#[test]
fn checkpoint_storage_refuses_symlinked_parents_files_and_shared_inodes() {
    for component in ["owner", "owner/terminal"] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        if component.contains('/') {
            std::fs::create_dir(root.path().join("owner")).unwrap();
            std::fs::set_permissions(root.path().join("owner"), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        symlink(outside.path(), root.path().join(component)).unwrap();
        assert!(TerminalCheckpointStore::prepare(root.path()).is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = TerminalCheckpointStore::prepare(root.path()).unwrap();
    let target = outside.path().join("held");
    std::fs::write(&target, b"outside untouched").unwrap();
    symlink(&target, checkpoint_path(root.path())).unwrap();
    let terminal = SessionTerminal::new(8);
    assert!(store.save(&terminal).is_err());
    assert!(store.load(8).is_err());
    assert!(store.clear().is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"outside untouched");
    std::fs::remove_file(checkpoint_path(root.path())).unwrap();
    store.save(&terminal).unwrap();
    std::fs::remove_file(&target).unwrap();
    std::fs::hard_link(checkpoint_path(root.path()), &target).unwrap();
    let prior = std::fs::read(&target).unwrap();
    terminal.publish(b"new bytes");
    assert!(store.save(&terminal).is_err());
    assert!(store.load(8).is_err());
    assert!(store.clear().is_err());
    assert_eq!(std::fs::read(&target).unwrap(), prior);
}

#[test]
fn checkpoint_io_stays_below_held_directory_after_parent_replacement() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = TerminalCheckpointStore::prepare(root.path()).unwrap();
    let retained = root.path().join("retained-owner");
    std::fs::rename(root.path().join("owner"), &retained).unwrap();
    symlink(outside.path(), root.path().join("owner")).unwrap();
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"retained");
    store.save(&terminal).unwrap();
    assert_eq!(
        store.load(8).unwrap().subscribe().1.render,
        terminal.subscribe().1.render
    );
    assert!(retained.join("terminal/checkpoint.msgpack").exists());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    store.clear().unwrap();
    assert!(!retained.join("terminal/checkpoint.msgpack").exists());
}

#[test]
fn checkpoint_storage_rejects_corruption_excess_bytes_and_permissive_files() {
    let root = tempfile::tempdir().unwrap();
    let store = TerminalCheckpointStore::prepare(root.path()).unwrap();
    store.save(&SessionTerminal::new(8)).unwrap();
    let path = checkpoint_path(root.path());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(store.load(8).is_err());
    assert!(store.save(&SessionTerminal::new(8)).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(crate::terminal::MAX_CHECKPOINT_BYTES as u64 + 1).unwrap();
    assert!(store.load(8).is_err());
    drop(file);
    std::fs::write(&path, b"truncated continuation").unwrap();
    assert!(store.load(8).is_err());
}
