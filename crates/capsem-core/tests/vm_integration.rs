//! Integration tests for VM lifecycle.
//!
//! These tests require:
//! - The `com.apple.security.virtualization` entitlement (code-signed binary)
//! - VM assets built by `images/build.py` in `assets/`
//!
//! Run with: CAPSEM_ASSETS_DIR=./assets cargo test --test vm_integration
//! They are skipped by default when assets are not present.

use std::path::PathBuf;

use capsem_core::VmConfig;

fn assets_dir() -> Option<PathBuf> {
    let dir = std::env::var("CAPSEM_ASSETS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("assets"));

    if dir.join("vmlinuz").exists() {
        Some(dir)
    } else {
        None
    }
}

fn make_config(assets: &std::path::Path) -> VmConfig {
    let mut builder = VmConfig::builder()
        .cpu_count(2)
        .ram_bytes(512 * 1024 * 1024)
        .kernel_path(assets.join("vmlinuz"));

    if assets.join("initrd.img").exists() {
        builder = builder.initrd_path(assets.join("initrd.img"));
    }
    if assets.join("rootfs.img").exists() {
        builder = builder.disk_path(assets.join("rootfs.img"));
    }

    builder.build().expect("VmConfig should be valid with real assets")
}

#[test]
fn vm_config_builds_with_real_assets() {
    let Some(assets) = assets_dir() else {
        eprintln!("SKIPPED: VM assets not found. Run images/build.py first.");
        return;
    };

    let config = make_config(&assets);
    assert_eq!(config.cpu_count, 2);
    assert_eq!(config.ram_bytes, 512 * 1024 * 1024);
    assert!(config.kernel_path.exists());
}

#[test]
fn vm_create_requires_entitlement() {
    // This test documents that VirtualMachine::create will fail without
    // the virtualization entitlement. When running under `cargo test`
    // (unsigned), it should return an error rather than crash.
    let Some(assets) = assets_dir() else {
        eprintln!("SKIPPED: VM assets not found.");
        return;
    };

    let config = make_config(&assets);

    // Without entitlement, create may fail with a validation error.
    // We just verify it doesn't panic/crash.
    let result = capsem_core::VirtualMachine::create(&config);
    match result {
        Ok((_vm, _rx, _input_fd)) => eprintln!("VM created (running with entitlement)"),
        Err(e) => eprintln!("VM creation failed as expected without entitlement: {e}"),
    }
}

#[test]
fn vm_has_socket_devices_after_create() {
    // Verify that VirtualMachine::create configures a vsock socket device.
    // When running with the virtualization entitlement, the VM should have
    // exactly one socket device for vsock communication.
    let Some(assets) = assets_dir() else {
        eprintln!("SKIPPED: VM assets not found.");
        return;
    };

    let config = make_config(&assets);

    match capsem_core::VirtualMachine::create(&config) {
        Ok((vm, _rx, _input_fd)) => {
            let socket_devices = vm.socket_devices();
            assert_eq!(
                socket_devices.count(), 1,
                "VM should have exactly one socket device configured"
            );
            eprintln!("VM has {} socket device(s)", socket_devices.count());
        }
        Err(e) => {
            eprintln!("SKIPPED: VM creation failed (no entitlement): {e}");
        }
    }
}

#[test]
fn vsock_manager_rejects_empty_devices() {
    // VsockManager::new should fail when given an empty socket devices array.
    use objc2_foundation::NSArray;
    use objc2_virtualization::VZSocketDevice;

    let empty_arr = NSArray::<VZSocketDevice>::from_retained_slice(&[]);
    let empty: &NSArray<VZSocketDevice> = &empty_arr;
    let result = capsem_core::VsockManager::new(
        empty,
        &[capsem_core::VSOCK_PORT_CONTROL, capsem_core::VSOCK_PORT_TERMINAL],
    );
    assert!(result.is_err(), "VsockManager should reject empty socket devices");
}
