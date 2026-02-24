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
