//! Checkpoint state encoding, restore validation, and restore activation.

use super::*;

#[test]
fn block_checkpoint_state_binds_open_fd_and_guest_contract() {
    let path = temp_disk("checkpoint-contract.img", 1000);
    let metadata = std::fs::metadata(&path).unwrap();
    let mut dev = VirtioBlockDevice::new(&path, false).unwrap();

    let state = dev.checkpoint_state().unwrap();

    assert_eq!(state.len(), BLOCK_CHECKPOINT_STATE_LEN);
    assert_eq!(&state[..8], b"CPSBLK\0\0");
    assert_eq!(
        u32::from_le_bytes(
            state[BLOCK_CHECKPOINT_VERSION_OFFSET..BLOCK_CHECKPOINT_DEVICE_OFFSET]
                .try_into()
                .unwrap()
        ),
        1
    );
    assert_eq!(checkpoint_u64(&state, BLOCK_CHECKPOINT_DEVICE_OFFSET), metadata.dev());
    assert_eq!(checkpoint_u64(&state, BLOCK_CHECKPOINT_INODE_OFFSET), metadata.ino());
    assert_eq!(checkpoint_u64(&state, BLOCK_CHECKPOINT_LENGTH_OFFSET), 1000);
    assert_eq!(checkpoint_u64(&state, BLOCK_CHECKPOINT_CAPACITY_OFFSET), 1);
    assert_eq!(state[BLOCK_CHECKPOINT_READ_ONLY_OFFSET], 0);
    assert_eq!(
        &state[BLOCK_CHECKPOINT_ID_OFFSET..],
        &dev.device_id,
        "checkpoint must bind the exact guest-visible 20-byte ID"
    );
}

#[test]
fn block_checkpoint_restore_binds_open_fd_not_path() {
    let path = temp_disk("checkpoint-open-fd.img", 4096);
    let old_path = path.with_extension("original");
    let _ = std::fs::remove_file(&old_path);
    let mut source = VirtioBlockDevice::new(&path, false).unwrap();
    let state = source.checkpoint_state().unwrap();
    let mut restored = VirtioBlockDevice::new(&path, false).unwrap();

    std::fs::rename(&path, &old_path).unwrap();
    temp_disk("checkpoint-open-fd.img", 8192);

    restored.restore_checkpoint_state(&state).unwrap();
}

#[test]
fn block_checkpoint_restore_rejects_same_size_replacement_inode() {
    let path = temp_disk("checkpoint-replacement.img", 4096);
    let old_path = path.with_extension("original");
    let _ = std::fs::remove_file(&old_path);
    let mut source = VirtioBlockDevice::new(&path, false).unwrap();
    let state = source.checkpoint_state().unwrap();

    std::fs::rename(&path, &old_path).unwrap();
    temp_disk("checkpoint-replacement.img", 4096);
    let mut restored = VirtioBlockDevice::new(&path, false).unwrap();

    let error = restored.restore_checkpoint_state(&state).unwrap_err();
    assert!(error.to_string().contains("identity"), "{error:#}");
    assert!(restored.queue.is_none() && restored.mem.is_none());
}

#[test]
fn block_checkpoint_restore_rejects_live_length_drift() {
    let path = temp_disk("checkpoint-length.img", 4096);
    let mut source = VirtioBlockDevice::new(&path, false).unwrap();
    let state = source.checkpoint_state().unwrap();
    let mut restored = VirtioBlockDevice::new(&path, false).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(8192)
        .unwrap();

    let error = restored.restore_checkpoint_state(&state).unwrap_err();
    assert!(error.to_string().contains("length"), "{error:#}");
    assert!(restored.queue.is_none() && restored.mem.is_none());
}

#[test]
fn block_checkpoint_restore_allows_same_length_content_changes() {
    let path = temp_disk("checkpoint-content.img", 4096);
    let mut source = VirtioBlockDevice::new(&path, false).unwrap();
    let state = source.checkpoint_state().unwrap();
    std::fs::write(&path, vec![0xa5; 4096]).unwrap();
    let mut restored = VirtioBlockDevice::new(&path, false).unwrap();

    restored.restore_checkpoint_state(&state).unwrap();
}

#[test]
fn block_checkpoint_rejects_open_fd_access_mode_drift() {
    let path = temp_disk("checkpoint-fd-mode.img", 4096);
    let mut dev = VirtioBlockDevice::new(&path, false).unwrap();
    dev.read_only = true;

    let error = dev.checkpoint_state().unwrap_err();

    assert!(error.to_string().contains("access mode"), "{error:#}");
}

#[test]
fn block_checkpoint_restore_rejects_read_only_role_drift() {
    let path = temp_disk("checkpoint-role.img", 4096);
    let mut source = VirtioBlockDevice::new(&path, false).unwrap();
    let state = source.checkpoint_state().unwrap();
    let mut restored = VirtioBlockDevice::new(&path, true).unwrap();

    let error = restored.restore_checkpoint_state(&state).unwrap_err();
    assert!(error.to_string().contains("read-only"), "{error:#}");
    assert!(restored.queue.is_none() && restored.mem.is_none());
}

#[test]
fn block_checkpoint_restore_rejects_guest_id_drift_for_same_inode() {
    let path = temp_disk("checkpoint-id-source.img", 4096);
    let alias = path.with_file_name("checkpoint-id-alias.img");
    let _ = std::fs::remove_file(&alias);
    crate::auditfs::link_test_fixture(&path, &alias).unwrap();
    let mut source = VirtioBlockDevice::new(&path, false).unwrap();
    let state = source.checkpoint_state().unwrap();
    let mut restored = VirtioBlockDevice::new(&alias, false).unwrap();

    let error = restored.restore_checkpoint_state(&state).unwrap_err();
    assert!(error.to_string().contains("device ID"), "{error:#}");
    assert!(restored.queue.is_none() && restored.mem.is_none());
}

#[test]
fn block_checkpoint_restore_rejects_capacity_drift() {
    let path = temp_disk("checkpoint-capacity.img", 4096);
    let mut source = VirtioBlockDevice::new(&path, false).unwrap();
    let mut state = source.checkpoint_state().unwrap();
    state[BLOCK_CHECKPOINT_CAPACITY_OFFSET..BLOCK_CHECKPOINT_READ_ONLY_OFFSET].copy_from_slice(&9_u64.to_le_bytes());
    let mut restored = VirtioBlockDevice::new(&path, false).unwrap();

    let error = restored.restore_checkpoint_state(&state).unwrap_err();
    assert!(error.to_string().contains("capacity"), "{error:#}");
    assert!(restored.queue.is_none() && restored.mem.is_none());
}

#[test]
fn block_checkpoint_restore_rejects_malformed_and_trailing_state() {
    let path = temp_disk("checkpoint-malformed.img", 4096);
    let mut source = VirtioBlockDevice::new(&path, false).unwrap();
    let valid = source.checkpoint_state().unwrap();
    let mut cases = Vec::new();
    cases.push((valid[..valid.len() - 1].to_vec(), "length"));
    let mut trailing = valid.clone();
    trailing.push(0);
    cases.push((trailing, "length"));
    let mut bad_magic = valid.clone();
    bad_magic[0] ^= 0xff;
    cases.push((bad_magic, "magic"));
    let mut bad_version = valid.clone();
    bad_version[BLOCK_CHECKPOINT_VERSION_OFFSET..BLOCK_CHECKPOINT_DEVICE_OFFSET].copy_from_slice(&2_u32.to_le_bytes());
    cases.push((bad_version, "version"));
    let mut bad_bool = valid;
    bad_bool[BLOCK_CHECKPOINT_READ_ONLY_OFFSET] = 2;
    cases.push((bad_bool, "boolean"));

    for (state, expected) in cases {
        let mut restored = VirtioBlockDevice::new(&path, false).unwrap();
        let error = restored.restore_checkpoint_state(&state).unwrap_err();
        assert!(error.to_string().contains(expected), "{error:#}");
        assert!(restored.queue.is_none() && restored.mem.is_none());
    }
}

#[test]
fn block_restore_activate_requires_validated_checkpoint_identity() {
    let path = temp_disk("checkpoint-activate-guard.img", 4096);
    let mut restored = VirtioBlockDevice::new(&path, false).unwrap();
    let mem = GuestMemory::new(1024 * 1024).unwrap();

    let error = restored
        .restore_activate(mem.clone_ref(RAM_BASE), &[warm_queue_config()])
        .unwrap_err();

    assert!(error.to_string().contains("checkpoint identity"), "{error:#}");
    assert!(restored.queue.is_none() && restored.mem.is_none());
}

#[test]
fn block_restore_activate_succeeds_after_identity_validation() {
    let path = temp_disk("checkpoint-activate.img", 4096);
    let mut source = VirtioBlockDevice::new(&path, false).unwrap();
    let state = source.checkpoint_state().unwrap();
    let mut restored = VirtioBlockDevice::new(&path, false).unwrap();
    restored.restore_checkpoint_state(&state).unwrap();
    let mem = GuestMemory::new(1024 * 1024).unwrap();

    restored
        .restore_activate(mem.clone_ref(RAM_BASE), &[warm_queue_config()])
        .unwrap();

    assert!(restored.queue.is_some());
    assert!(restored.mem.is_some());
}

#[test]
fn block_restore_activate_revalidates_open_fd_before_queue_activation() {
    let path = temp_disk("checkpoint-activate-revalidate.img", 4096);
    let mut source = VirtioBlockDevice::new(&path, false).unwrap();
    let state = source.checkpoint_state().unwrap();
    let mut restored = VirtioBlockDevice::new(&path, false).unwrap();
    restored.restore_checkpoint_state(&state).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(8192)
        .unwrap();
    let mem = GuestMemory::new(1024 * 1024).unwrap();

    let error = restored
        .restore_activate(mem.clone_ref(RAM_BASE), &[warm_queue_config()])
        .unwrap_err();

    assert!(error.to_string().contains("length"), "{error:#}");
    assert!(restored.queue.is_none() && restored.mem.is_none());
}

// STATUS=0 from the guest: the worker must stop and release its rings, and a
// second DRIVER_OK must spawn a fresh worker on the new rings. Before, the
// transport reset only itself: a second activation spawned a second worker on
// the same eventfd and the first kept running against freed guest memory.
#[cfg(target_os = "linux")]
#[test]
fn block_reset_stops_the_worker_and_allows_reactivation() {
    let path = temp_disk("reset-reactivate.img", 512);
    let mut h = TestHarness::new_with_async_notify(&path, false);
    assert!(h.dev.worker_handle.is_some());

    h.dev.reset();
    assert!(h.dev.worker_handle.is_none() && h.dev.control_tx.is_none());
    assert!(h.dev.queue.is_none() && h.dev.mem.is_none());

    let queue_config = || QueueConfig {
        desc_addr: RAM_BASE + DESC_TABLE_OFFSET,
        driver_addr: RAM_BASE + AVAIL_RING_OFFSET,
        device_addr: RAM_BASE + USED_RING_OFFSET,
        size: QUEUE_TEST_SIZE,
        warm_restore: false,
        event_idx: false,
    };
    h.dev.activate(h.mem.clone_ref(RAM_BASE), &[queue_config()]);
    assert!(h.dev.worker_handle.is_some(), "a fresh worker after reset");

    // A second activation without a reset is ignored, not doubled.
    h.dev.activate(h.mem.clone_ref(RAM_BASE), &[queue_config()]);
    assert!(h.dev.worker_handle.is_some());
    h.dev.reset();
    assert!(h.dev.worker_handle.is_none());
}
