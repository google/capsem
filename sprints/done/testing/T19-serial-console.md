# Sprint T19: Serial Console
## Goal
Un-skip serial console tests, add boot timing regression gate

## Files
- tests/capsem-serial/conftest.py
- tests/capsem-serial/test_serial_log.py
- tests/capsem-serial/test_boot_timing.py
- tests/capsem-service/test_svc_logs.py (modify)

## Tasks
- [x] test_serial_log_exists: boot VM, GET /logs/{id} non-empty, contains Linux kernel output
- [x] test_boot_timing: measure provision to exec-ready, assert < 30s
- [x] test_serial_log_before_delete: verify logs captured before VM deletion
- [x] Remove @pytest.mark.skip from test_svc_logs.py
- [x] Marked serial

## Verification
pytest tests/capsem-serial/ -m serial passes

## Depends On
T14
