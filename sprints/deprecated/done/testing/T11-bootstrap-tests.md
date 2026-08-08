# Sprint T11: Bootstrap and Setup Tests

## Goal

Validate the setup and install flow: `just doctor` checks, asset presence verification, initrd format correctness, cross-compilation output, and manifest/hash integrity.

## Files

```
tests/capsem-bootstrap/
    test_doctor.py
    test_assets.py
    test_initrd.py
    test_cross_compile.py
```

Marker: `bootstrap`

## Tasks

### Doctor (`test_doctor.py`)
- [ ] Run `just doctor` on a fully set up machine, verify it succeeds
- [ ] Verify doctor creates the `.dev-setup` sentinel file
- [ ] Remove rustup from PATH, verify doctor fails with clear error
- [ ] Remove docker/colima from PATH, verify doctor fails with clear error

### Assets (`test_assets.py`)
- [ ] Run `_check-assets` with missing kernel, verify it errors with descriptive message
- [ ] Run `_check-assets` with all assets present, verify success

### Initrd (`test_initrd.py`)
- [ ] Verify the built initrd is valid gzip (gunzip test succeeds)
- [ ] Verify the initrd contains a valid cpio archive
- [ ] Extract initrd and verify all injected binaries have 555 permissions

### Manifest and Hashes (`test_cross_compile.py`)
- [ ] Verify the asset manifest is valid JSON
- [ ] Verify the manifest version matches the version in Cargo.toml
- [ ] Run `file` on cross-compiled guest binary, verify correct ELF architecture (aarch64/x86_64)
- [ ] Verify the guest binary is statically linked (not dynamically linked)
- [ ] Verify B3SUMS file entries match actual blake3 hashes of built assets

## Verification

```bash
pytest tests/capsem-bootstrap/ -m bootstrap -v
```

All tests green. The bootstrap pipeline produces correct, well-formed, and integrity-verified artifacts.

## Depends On

None (tests the build and setup pipeline directly).
