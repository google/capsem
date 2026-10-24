use super::*;

fn fixture() -> (Arc<ServiceState>, tempfile::TempDir, PersistentVmEntry) {
    let (state, dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let session_dir = state.run_dir.join("persistent/pinned-assets");
    capsem_core::create_virtiofs_session(&session_dir, 1).unwrap();
    let entry = test_persistent_entry("pinned-assets", session_dir);
    (state, dir, entry)
}

fn plain_assets(state: &ServiceState) {
    let arch = state.assets_dir.join(host_manifest_arch());
    for (name, body) in TEST_RUNTIME_ASSETS {
        std::fs::rename(arch.join(hash_filename(name, &test_asset_hex(body))), arch.join(name)).unwrap();
    }
}

#[test]
fn persistent_plain_assets_require_each_full_recorded_digest_to_match_manifest() {
    for kind in ["kernel", "initrd", "rootfs"] {
        let (state, _dir, mut entry) = fixture();
        plain_assets(&state);
        assert!(state.persistent_entry_resume_state_cached(&entry).1);
        let pin = match kind {
            "kernel" => &mut entry.asset_pins.kernel,
            "initrd" => &mut entry.asset_pins.initrd,
            _ => &mut entry.asset_pins.rootfs,
        };
        // Retaining the hash-filename prefix must not hide a full-pin mismatch.
        let last = pin.hash.len() - 1;
        let replacement = if pin.hash.ends_with('a') { "b" } else { "a" };
        pin.hash.replace_range(last.., replacement);
        let (status, available, reason) = state.persistent_entry_resume_state_cached(&entry);
        assert_eq!(status, VmLifecycleState::Stopped);
        assert!(!available, "{kind}: current plain files are not the recorded image");
        assert!(reason.unwrap().contains(kind));
    }
}

#[test]
fn persistent_plain_assets_fail_closed_without_manifest_binding_and_recover_on_repair() {
    let (state, _dir, entry) = fixture();
    plain_assets(&state);
    assert!(state.persistent_entry_resume_state_cached(&entry).1);
    let manifest = state.assets_dir.join("manifest.json");
    for bytes in [b"{}".as_slice(), b"not-json".as_slice()] {
        std::fs::write(&manifest, bytes).unwrap();
        assert!(!state.persistent_entry_resume_state_cached(&entry).1);
    }
    std::fs::remove_file(&manifest).unwrap();
    assert!(!state.persistent_entry_resume_state_cached(&entry).1);
    install_test_runtime_manifest(&state);
    assert!(state.persistent_entry_resume_state_cached(&entry).1);
}

#[test]
fn persistent_hash_named_assets_do_not_require_current_runtime_or_plain_file_binding() {
    let (state, _dir, entry) = fixture();
    std::fs::remove_file(state.assets_dir.join("manifest.json")).unwrap();
    *state.manifest.write().unwrap() = None;
    assert!(state.persistent_entry_resume_state_cached(&entry).1);
    let path = boot_asset_pin_path(&state.assets_dir, host_manifest_arch(), &entry.asset_pins.initrd);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        !state.persistent_entry_resume_state_cached(&entry).1,
        "directory is not a boot asset"
    );
}

#[test]
fn persistent_plain_assets_track_parent_manifest_replacement_and_cached_repairs() {
    let (state, _dir, mut entry) = fixture();
    plain_assets(&state);
    // Both accepted digest spellings are the same pin, not different runtimes.
    entry.asset_pins.initrd.hash = entry
        .asset_pins
        .initrd
        .hash
        .strip_prefix("blake3:")
        .unwrap()
        .to_string();
    let local = state.assets_dir.join("manifest.json");
    let parent = state.assets_dir.parent().unwrap().join("manifest.json");
    std::fs::rename(&local, &parent).unwrap();
    assert!(state.persistent_entry_resume_state_cached(&entry).1);
    let original = std::fs::read(&parent).unwrap();
    let mut manifest = ManifestV2::from_json(std::str::from_utf8(&original).unwrap()).unwrap();
    manifest
        .assets
        .releases
        .get_mut(TEST_ASSET_VERSION)
        .unwrap()
        .arches
        .get_mut(host_manifest_arch())
        .unwrap()
        .get_mut("vmlinuz")
        .unwrap()
        .hash = "a".repeat(64);
    std::fs::write(&parent, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(
        !state.persistent_entry_resume_state_cached(&entry).1,
        "stale in-memory manifest must not bind a plain file"
    );
    std::fs::write(&parent, original).unwrap();
    assert!(state.persistent_entry_resume_state_cached(&entry).1);
}

#[test]
fn persistent_plain_asset_warm_availability_reads_stay_metadata_only() {
    let (state, _dir, entry) = fixture();
    plain_assets(&state);
    assert!(state.persistent_entry_resume_state_cached(&entry).1);
    let started = std::time::Instant::now();
    for _ in 0..200 {
        assert!(std::hint::black_box(state.persistent_entry_resume_state_cached(&entry)).1);
    }
    // Diagnostic timing, not a noisy CI performance threshold or native claim.
    eprintln!("warm persistent availability mean: {:?}", started.elapsed() / 200);
}
