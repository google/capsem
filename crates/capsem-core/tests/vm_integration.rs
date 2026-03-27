//! Integration tests for VM lifecycle.
//!
//! These tests require:
//! - The `com.apple.security.virtualization` entitlement (code-signed binary)
//! - VM assets built by `images/build.py` in `assets/`
//!
//! Run with: CAPSEM_ASSETS_DIR=./assets cargo test --test vm_integration
//! They are skipped by default when assets are not present.

use std::path::PathBuf;

use capsem_core::{Hypervisor, VmConfig, VmHandle, SerialConsole};

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
    if assets.join("rootfs.squashfs").exists() {
        builder = builder.disk_path(assets.join("rootfs.squashfs"));
    }

    builder.build().expect("VmConfig should be valid with real assets")
}

// -----------------------------------------------------------------------
// Config validation (no VM assets needed)
// -----------------------------------------------------------------------

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

// -----------------------------------------------------------------------
// Hypervisor trait surface (no assets needed)
// -----------------------------------------------------------------------

#[test]
fn hypervisor_trait_is_object_safe() {
    fn _assert(_: &dyn Hypervisor) {}
    let h = capsem_core::AppleVzHypervisor;
    _assert(&h);
}

#[test]
fn vm_handle_trait_is_object_safe() {
    fn _assert(_: &dyn VmHandle) {}
}

#[test]
fn serial_console_trait_is_object_safe() {
    fn _assert(_: &dyn SerialConsole) {}
}

#[test]
fn hypervisor_boot_fails_with_missing_kernel() {
    let h = capsem_core::AppleVzHypervisor;
    let config = VmConfig::builder()
        .kernel_path("/nonexistent/vmlinuz");
    // Should fail at build() -- missing kernel
    assert!(config.build().is_err());
}

#[test]
fn hypervisor_boot_fails_with_fake_kernel() {
    let tmp = tempfile::tempdir().unwrap();
    let kernel = tmp.path().join("vmlinuz");
    std::fs::write(&kernel, b"not a real kernel").unwrap();

    let config = VmConfig::builder()
        .kernel_path(&kernel)
        .build()
        .unwrap();

    let h = capsem_core::AppleVzHypervisor;
    let ports = [capsem_core::VSOCK_PORT_CONTROL, capsem_core::VSOCK_PORT_TERMINAL];
    let result = h.boot(&config, &ports);
    // Fails gracefully (no entitlement or invalid kernel), does not panic
    assert!(result.is_err(), "boot with fake kernel should fail");
}

#[test]
fn hypervisor_boot_with_empty_ports() {
    let tmp = tempfile::tempdir().unwrap();
    let kernel = tmp.path().join("vmlinuz");
    std::fs::write(&kernel, b"fake").unwrap();

    let config = VmConfig::builder()
        .kernel_path(&kernel)
        .build()
        .unwrap();

    let h = capsem_core::AppleVzHypervisor;
    // Empty ports array -- should still fail (no entitlement), not panic
    let result = h.boot(&config, &[]);
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// VsockConnection
// -----------------------------------------------------------------------

#[test]
fn vsock_connection_can_be_sent_to_thread() {
    let conn = capsem_core::VsockConnection::new(42, 5001, Box::new(()));
    let handle = std::thread::spawn(move || {
        assert_eq!(conn.fd, 42);
        assert_eq!(conn.port, 5001);
    });
    handle.join().unwrap();
}

#[test]
fn vsock_connection_can_be_stored_in_vec() {
    let mut conns = Vec::new();
    for i in 0..10 {
        conns.push(capsem_core::VsockConnection::new(i, 5000 + i as u32, Box::new(())));
    }
    assert_eq!(conns.len(), 10);
    assert_eq!(conns[5].fd, 5);
    assert_eq!(conns[5].port, 5005);
}

// -----------------------------------------------------------------------
// Entitlement-gated tests (assets required)
// -----------------------------------------------------------------------

#[test]
fn hypervisor_boot_requires_entitlement() {
    let Some(assets) = assets_dir() else {
        eprintln!("SKIPPED: VM assets not found.");
        return;
    };

    let config = make_config(&assets);
    let vsock_ports = [
        capsem_core::VSOCK_PORT_CONTROL,
        capsem_core::VSOCK_PORT_TERMINAL,
        capsem_core::VSOCK_PORT_SNI_PROXY,
        capsem_core::VSOCK_PORT_MCP_GATEWAY,
    ];

    let result = capsem_core::AppleVzHypervisor.boot(&config, &vsock_ports);
    match result {
        Ok((vm, _rx)) => {
            // If we get here, we have the entitlement (e.g. just run)
            eprintln!("VM booted (running with entitlement)");
            // Verify the handle works
            let _state = vm.state();
            let _serial = vm.serial();
            let _ = vm.stop();
        }
        Err(e) => eprintln!("VM boot failed as expected without entitlement: {e}"),
    }
}

#[test]
fn hypervisor_boot_returns_vsock_receiver() {
    let Some(assets) = assets_dir() else {
        eprintln!("SKIPPED: VM assets not found.");
        return;
    };

    let config = make_config(&assets);
    let ports = [capsem_core::VSOCK_PORT_CONTROL, capsem_core::VSOCK_PORT_TERMINAL];

    match capsem_core::AppleVzHypervisor.boot(&config, &ports) {
        Ok((_vm, mut rx)) => {
            // The receiver should exist and be empty (no guest connected yet)
            assert!(rx.try_recv().is_err());
        }
        Err(_) => eprintln!("SKIPPED: no entitlement"),
    }
}
