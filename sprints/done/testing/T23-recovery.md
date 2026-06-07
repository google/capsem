# Sprint T23: Recovery
## Goal
Verify system handles stale state and recovers from crashes

## Files
- tests/capsem-recovery/conftest.py
- tests/capsem-recovery/test_stale_socket.py
- tests/capsem-recovery/test_stale_instances.py
- tests/capsem-recovery/test_orphaned_process.py
- tests/capsem-recovery/test_double_service.py

## Tasks
- [x] test_stale_socket: create fake socket file, start service, should replace it and bind
- [x] test_stale_instance_sockets: create fake .sock in instances/, start service, list shows stale entries with pid=0
- [x] test_orphaned_process: start VM, kill service (not VM process), restart service, stale VM in list, delete cleans up
- [x] test_double_service: start service A, try service B on same socket, B fails with clear error, A still works
- [x] Marked recovery

## Verification
pytest tests/capsem-recovery/ -m recovery

## Depends On
T14
