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

#[test]
fn rollback_invalidates_completion_even_when_the_native_slot_is_not_removable() {
    let root = session();
    let files = Arc::new(Files::prepare(root.path()).unwrap());
    std::fs::create_dir(files.path()).unwrap();
    std::fs::write(files.marker_path(), b"old completion").unwrap();
    assert!(files.begin().is_err());
    files.invalidate().unwrap();
    assert!(!files.marker_path().exists());
    assert!(files.path().is_dir());
}

struct RecoveringVm {
    state: std::cell::Cell<capsem_core::hypervisor::VmState>,
    resumes: std::cell::Cell<usize>,
    fail: bool,
    stays_paused: bool,
}

impl capsem_core::hypervisor::VmHandle for RecoveringVm {
    fn stop(&self) -> Result<()> {
        panic!("rollback must not stop the original VM")
    }
    fn state(&self) -> capsem_core::hypervisor::VmState {
        self.state.get()
    }
    fn serial(&self) -> &dyn capsem_core::hypervisor::SerialConsole {
        unreachable!()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn resume(&self) -> Result<()> {
        self.resumes.set(self.resumes.get() + 1);
        anyhow::ensure!(!self.fail, "fixture resume rejected");
        if !self.stays_paused {
            self.state.set(capsem_core::hypervisor::VmState::Running);
        }
        Ok(())
    }
}

#[test]
fn rollback_resumes_only_paused_vm_and_refuses_failed_or_incomplete_recovery() {
    use capsem_core::hypervisor::VmState;
    for (state, fail, stays_paused, success, resumes) in [
        (VmState::Running, false, false, true, 0),
        (VmState::Paused, false, false, true, 1),
        (VmState::Paused, true, false, false, 1),
        (VmState::Paused, false, true, false, 1),
        (VmState::Stopped, false, false, false, 0),
    ] {
        let vm = RecoveringVm {
            state: std::cell::Cell::new(state),
            resumes: std::cell::Cell::new(0),
            fail,
            stays_paused,
        };
        assert_eq!(resume_running_vm(&vm).is_ok(), success);
        assert_eq!(vm.resumes.get(), resumes);
    }
}
