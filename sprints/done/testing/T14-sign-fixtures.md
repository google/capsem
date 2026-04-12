# Sprint T14: Sign Fixtures
## Goal
Fix test fixtures to sign capsem-process before boot. CRITICAL -- unblocks every VM test.

## Files
- tests/helpers/sign.py (new)
- tests/helpers/service.py (modify)
- tests/capsem-mcp/conftest.py (modify)

## Tasks
- [ ] Add _sign_binary() helper to tests/helpers/sign.py
- [ ] Call sign_binary() in ServiceInstance.start() before subprocess.Popen
- [ ] Sign capsem-process and capsem-service
- [ ] Skip signing on Linux (KVM doesn't need entitlements)
- [ ] Pre-flight check: signing fails -> raise clear error not silent VM failure
- [ ] Update capsem-mcp conftest to sign before spawning
- [ ] Add verify_signed() helper
- [ ] Add ensure_all_signed() convenience function

## Verification
just test-mcp boots VMs on macOS

## Depends On
T0
