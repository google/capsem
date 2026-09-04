use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

// -----------------------------------------------------------------------
// Trait object safety (compile-time checks)
// -----------------------------------------------------------------------

fn _assert_object_safe(_h: &dyn Hypervisor, _v: &dyn VmHandle, _s: &dyn SerialConsole) {}

struct DummyKvmHandle;
impl VmHandle for DummyKvmHandle {
    fn stop(&self) -> Result<()> {
        Ok(())
    }
    fn state(&self) -> crate::vm::VmState {
        crate::vm::VmState::Stopped
    }
    fn serial(&self) -> &dyn SerialConsole {
        unimplemented!()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn kvm_default_impls_return_errors() {
    let handle = DummyKvmHandle;
    assert!(!handle.supports_checkpoint());
    assert!(handle.pause().is_err());
    assert!(handle.resume().is_err());
    assert!(handle.save_state(std::path::Path::new("")).is_err());
}

fn _assert_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<VsockConnection>();
    assert_sync::<VsockConnection>();
    assert_send::<Box<dyn VmHandle>>();
    assert_send::<Box<dyn Hypervisor>>();
    assert_sync::<Box<dyn Hypervisor>>();
}

// -----------------------------------------------------------------------
// VsockConnection -- construction
// -----------------------------------------------------------------------

#[test]
fn vsock_connection_preserves_fields() {
    let conn = VsockConnection::new(42, 5001, Box::new(()));
    assert_eq!(conn.raw_fd(), 42);
    assert_eq!(conn.port, 5001);
}

#[test]
fn vsock_connection_zero_fd() {
    // fd 0 (stdin) is a valid fd value
    let conn = VsockConnection::new(0, 5000, Box::new(()));
    assert_eq!(conn.raw_fd(), 0);
}

#[test]
fn vsock_connection_negative_fd() {
    // -1 represents an invalid fd, but the struct should still hold it
    let conn = VsockConnection::new(-1, 5000, Box::new(()));
    assert_eq!(conn.raw_fd(), -1);
}

#[test]
fn vsock_connection_max_port() {
    let conn = VsockConnection::new(10, u32::MAX, Box::new(()));
    assert_eq!(conn.port, u32::MAX);
}

// -----------------------------------------------------------------------
// VsockConnection -- lifetime anchor semantics
// -----------------------------------------------------------------------

#[test]
fn vsock_connection_anchor_is_dropped() {
    let dropped = Arc::new(AtomicBool::new(false));
    let dropped_clone = Arc::clone(&dropped);

    struct DropGuard(Arc<AtomicBool>);
    impl Drop for DropGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let conn = VsockConnection::new(10, 5000, Box::new(DropGuard(dropped_clone)));
    assert!(!dropped.load(Ordering::SeqCst));
    drop(conn);
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn vsock_connection_anchor_drop_order() {
    // Verify the anchor is dropped when connection is dropped,
    // even with complex anchor types (Vec, String, etc.)
    let counter = Arc::new(AtomicUsize::new(0));

    struct CountGuard(Arc<AtomicUsize>);
    impl Drop for CountGuard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let c = Arc::clone(&counter);
    let conn1 = VsockConnection::new(1, 5000, Box::new(CountGuard(c)));
    let c = Arc::clone(&counter);
    let conn2 = VsockConnection::new(2, 5001, Box::new(CountGuard(c)));

    assert_eq!(counter.load(Ordering::SeqCst), 0);
    drop(conn1);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    drop(conn2);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[test]
fn vsock_connection_unit_anchor() {
    // () anchor is the lightest possible -- verify it works
    let conn = VsockConnection::new(99, 5002, Box::new(()));
    assert_eq!(conn.raw_fd(), 99);
    drop(conn); // should not panic
}

#[test]
fn vsock_connection_string_anchor() {
    // Any Send type should work as anchor
    let conn = VsockConnection::new(5, 5002, Box::new(String::from("keep-alive")));
    assert_eq!(conn.port, 5002);
}

#[test]
fn vsock_connection_vec_anchor() {
    let data: Vec<u8> = vec![1, 2, 3, 4];
    let conn = VsockConnection::new(7, 5001, Box::new(data));
    assert_eq!(conn.raw_fd(), 7);
}

// -----------------------------------------------------------------------
// VsockConnection -- can be moved across threads
// -----------------------------------------------------------------------

#[test]
fn vsock_connection_send_across_thread() {
    let conn = VsockConnection::new(42, 5001, Box::new(()));
    let handle = std::thread::spawn(move || {
        assert_eq!(conn.raw_fd(), 42);
        assert_eq!(conn.port, 5001);
    });
    handle.join().unwrap();
}

#[test]
fn vsock_connection_shared_across_threads() {
    let conn = Arc::new(VsockConnection::new(42, 5001, Box::new(())));
    let conn2 = Arc::clone(&conn);
    let handle = std::thread::spawn(move || {
        assert_eq!(conn2.raw_fd(), 42);
    });
    assert_eq!(conn.port, 5001);
    handle.join().unwrap();
}

// -----------------------------------------------------------------------
// VmState re-export
// -----------------------------------------------------------------------

#[test]
fn vmstate_reexported_from_hypervisor() {
    // VmState should be accessible through the hypervisor module
    let state = VmState::Running;
    assert_eq!(state.as_str(), "running");
}
