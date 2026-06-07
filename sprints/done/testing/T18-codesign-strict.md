# Sprint T18: Codesign Strict
## Goal
Codesigning tests that FAIL not skip when binaries are unsigned

## Files
- tests/capsem-codesign/conftest.py
- tests/capsem-codesign/test_process_signed.py
- tests/capsem-codesign/test_unsigned_boot_fails.py
- tests/capsem-codesign/test_all_binaries_signed.py

## Tasks
- [x] test_process_signed: codesign --verify capsem-process returns 0, FAIL if not (not skip), verify virtualization entitlement present
- [x] test_unsigned_boot_fails: strip signature from copy, provision VM, verify error mentions entitlement/virtualization, must not crash/hang
- [x] test_all_binaries_signed: capsem-process, capsem-service, capsem, capsem-mcp all signed
- [x] Marked codesign, macOS only

## Verification
pytest tests/capsem-codesign/ -m codesign passes on macOS

## Depends On
T14
