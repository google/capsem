# Sprint T10: Security Invariant Tests

## Goal

Verify every security invariant: guest binary permissions, read-only rootfs, network isolation, VirtioFS sandboxing, codesigning, asset integrity, environment variable blocklist, and protocol frame limits.

## Files

```
tests/capsem-security/
    conftest.py
    test_binary_perms.py
    test_rootfs.py
    test_network_isolation.py
    test_virtiofs.py
    test_codesigning.py
    test_asset_integrity.py
    test_env_blocklist.py
    test_frame_limits.py
```

Marker: `security`

## Tasks

### Binary Permissions (`test_binary_perms.py`)
- [ ] Stat all guest binaries after initrd repack, verify mode is 555
- [ ] Attempt chmod on a guest binary inside the VM, verify it fails

### Rootfs (`test_rootfs.py`)
- [ ] Verify `mount` output shows rootfs mounted read-only
- [ ] Attempt `touch /bin/test_file`, verify permission denied
- [ ] Attempt `rm /bin/ls`, verify permission denied
- [ ] Verify overlay upper directory is tmpfs (not persistent)

### Network Isolation (`test_network_isolation.py`)
- [ ] Verify only `lo` and `dummy0` interfaces exist in the guest
- [ ] Verify no default external route exists
- [ ] Verify iptables REDIRECT rule is in place for traffic interception
- [ ] Attempt DNS query for a spoofed domain, verify correct handling
- [ ] Verify allowed domains are reachable through the proxy
- [ ] Verify blocked domains fail with clear error

### VirtioFS (`test_virtiofs.py`)
- [ ] Create a symlink pointing outside the workspace, verify escape is blocked
- [ ] Attempt path traversal with `../../../etc/passwd`, verify failure
- [ ] Verify no host files outside the workspace are accessible from the guest

### Codesigning (`test_codesigning.py`)
- [ ] Verify `com.apple.security.virtualization` entitlement is present in the signed binary
- [ ] Attempt to run an unsigned binary with VZ calls, verify it fails
- [ ] Verify entitlements plist is valid XML

### Asset Integrity (`test_asset_integrity.py`)
- [ ] Supply a truncated kernel image, verify boot rejects it
- [ ] Verify asset hashes match the manifest (B3SUMS)
- [ ] Tamper with the manifest file, verify detection at boot

### Environment Blocklist (`test_env_blocklist.py`)
- [ ] Verify `LD_PRELOAD` is not injected into guest environment
- [ ] Verify `LD_LIBRARY_PATH` is not injected into guest environment
- [ ] Verify `BASH_FUNC_*` variables are not injected
- [ ] Verify `IFS` is not injected or overridden

### Frame Limits (`test_frame_limits.py`)
- [ ] Send a vsock frame larger than 256KB, verify it is rejected
- [ ] Send a vsock frame exactly 256KB, verify it is accepted

### Infrastructure (`conftest.py`)
- [ ] Create shared fixture: booted VM with full security config
- [ ] Register `security` pytest marker
- [ ] Helper to exec commands in guest and capture output

## Verification

```bash
pytest tests/capsem-security/ -m security -v
```

All tests green. Every security invariant from CLAUDE.md is mechanically verified.

## Depends On

None (tests security invariants directly).
