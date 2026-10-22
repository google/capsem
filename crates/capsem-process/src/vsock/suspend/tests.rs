use super::*;
use capsem_core::terminal::SessionTerminal;

fn session() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("system")).unwrap();
    std::fs::write(root.path().join("system/rootfs.img"), b"overlay").unwrap();
    root
}

#[test]
fn completion_requires_terminal_capture_and_vm_state_and_restore_consumes_the_pair() {
    let root = session();
    let files = Arc::new(Files::prepare(root.path()).unwrap());
    files.begin().unwrap();
    assert!(files.load_terminal().is_err());
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"primary\x1b[?1049halt\x07\x1b]0;Suspended\x07");
    let saved = files.capture(&terminal).unwrap();
    assert!(!files.marker_path().exists());
    assert!(saved.complete().is_err(), "missing VM state became resumable");
    std::fs::write(files.path(), b"native VM state").unwrap();
    files.capture(&terminal).unwrap().complete().unwrap();
    assert_eq!(std::fs::read(files.marker_path()).unwrap(), b"ok\n");
    let restored = files.load_terminal().unwrap();
    assert_eq!(terminal.subscribe().1.state, restored.subscribe().1.state);
    assert_eq!(terminal.subscribe().1.render, restored.subscribe().1.render);
    files.consume_restored().unwrap();
    assert!(!files.marker_path().exists());
    assert!(!files.path().exists());
    assert!(files.load_terminal().is_err());
    assert_eq!(restored.subscribe().1.state.bell_count, 1);
}

#[test]
fn beginning_new_checkpoint_durably_invalidates_a_previous_complete_pair() {
    let root = session();
    let files = Arc::new(Files::prepare(root.path()).unwrap());
    let terminal = SessionTerminal::new(8);
    std::fs::write(files.path(), b"old VM").unwrap();
    files.capture(&terminal).unwrap().complete().unwrap();
    files.begin().unwrap();
    assert!(!files.marker_path().exists());
    assert!(!files.path().exists());
    // Capturing new terminal bytes alone cannot resurrect the old pair.
    terminal.publish(b"new terminal");
    let saved = files.capture(&terminal).unwrap();
    assert!(!files.marker_path().exists());
    assert!(saved.complete().is_err());
    assert!(!files.marker_path().exists());
}

#[test]
fn linked_native_state_cannot_publish_a_completion_marker() {
    let root = session();
    let outside = tempfile::tempdir().unwrap();
    let files = Arc::new(Files::prepare(root.path()).unwrap());
    let target = outside.path().join("held");
    std::fs::write(&target, b"outside unchanged").unwrap();
    std::os::unix::fs::symlink(&target, files.path()).unwrap();
    assert!(files.capture(&SessionTerminal::new(8)).unwrap().complete().is_err());
    assert!(!files.marker_path().exists());
    assert_eq!(std::fs::read(&target).unwrap(), b"outside unchanged");
    std::fs::remove_file(files.path()).unwrap();
    std::fs::hard_link(&target, files.path()).unwrap();
    assert!(files.capture(&SessionTerminal::new(8)).unwrap().complete().is_err());
    assert!(!files.marker_path().exists());
}
