---
name: dev-testing-hypervisor
description: Testing the Capsem hypervisor layer -- Apple VZ (macOS) and KVM (Linux) backends. Use when writing or running tests for VM configuration, VirtioFS FUSE operations, vsock, serial console, virtio devices, or the hypervisor abstraction traits. Covers unit tests, integration tests, KVM CI, and what each backend needs.
---

# Hypervisor Testing

## Architecture

The hypervisor module (`crates/capsem-core/src/hypervisor/`) has:
- **Traits**: `Hypervisor`, `VmHandle`, `SerialConsole` in `mod.rs`
- **Apple VZ backend**: `apple_vz/` -- macOS only, uses Virtualization.framework
- **KVM backend**: `kvm/` -- Linux only, uses rust-vmm crates

Tests must cover both backends where possible. macOS CI tests Apple VZ, Linux CI (ubuntu-24.04-arm with /dev/kvm) tests KVM.

## Unit tests

VirtioFS FUSE operations have 30+ unit tests in `kvm/virtio_fs/mod.rs`:
- File I/O: open, read, write, create, release, flush, fsync, lseek
- Directory ops: opendir, readdir, mkdir, rmdir, unlink, rename, symlink, link
- Metadata: lookup, getattr, setattr, statfs, forget
- Adversarial: path traversal, truncated requests, invalid opcodes

Run them:
```bash
cargo test -p capsem-core virtio_fs    # VirtioFS tests only
cargo test -p capsem-core hypervisor   # All hypervisor tests
```

On macOS these run the KVM module's pure-logic tests (FUSE parsing, FDT generation) but skip anything that needs /dev/kvm. On Linux CI, all tests run including KVM integration.

## Integration tests

Cross-crate VM lifecycle tests in `crates/capsem-core/tests/`:
```bash
cargo test -p capsem-core --test '*'   # All integration tests
```

These test the full boot path: config validation, device setup, serial output, vsock handshake. They require VM assets to be built.

## CI setup

### macOS (ci.yaml, test job)
- Tests capsem-core, capsem-agent, capsem-logger, capsem-proto
- Cross-compile check for aarch64 + x86_64 musl targets
- No VM boot (no VZ entitlement in CI)

### Linux (ci.yaml, test-linux job)
- Runs on `ubuntu-24.04-arm` with KVM enabled
- Tests capsem-core, capsem-logger, capsem-proto (KVM backend compiles + tests)
- Verifies /dev/kvm is available (fails CI if KVM tests were silently skipped)

## What to test when changing hypervisor code

| Change | Tests to run |
|--------|-------------|
| VirtioFS FUSE ops | `cargo test virtio_fs` + `just run "capsem-doctor -k virtiofs"` |
| VM config / boot | `cargo test -p capsem-core` + `just run` (verify boot succeeds) |
| Vsock / serial | `cargo test -p capsem-core` + `just run "echo ok"` (verify I/O works) |
| KVM device model | `cargo test -p capsem-core` (Linux CI validates) |
| Hypervisor traits | `cargo test -p capsem-core` on both macOS and Linux CI |

## Rust async reference

Read `references/rust-async-patterns.md` for tokio patterns (tasks, channels, streams, error handling). Relevant for vsock, MITM proxy, and VirtioFS async worker code.

## Security invariants to test

- VirtioFS path traversal: FUSE lookup must reject `..` components
- Resource limits: file handle cap (4096), read size clamp (1MB), gather buffer limit (2MB)
- Read-only rootfs: squashfs lower layer must not be writable through overlay
- Guest binary integrity: binaries deployed chmod 555, guest cannot modify them
